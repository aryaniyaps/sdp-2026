"""Generate silver extraction labels with the teacher model (Azure gpt-5.6-sol).

Teacher runs once, offline. The student SLM trained on its output is what ships
in the memory engine, so nothing here is on the runtime path.

  python gen_labels.py --limit 0 --concurrency 6
"""

import argparse
import asyncio
import json
import random
import re
import sys
import time
from pathlib import Path

from openai import AsyncAzureOpenAI

from schema import EXTRACTION_SCHEMA, SYSTEM_PROMPT, build_user_prompt

ROOT = Path(__file__).parent
API_VERSION = "2025-04-01-preview"
WINDOW = 8  # turns per extraction window
# conversations 8,9 are held out so the eval split shares no speakers with train
HELDOUT_CONVS = {8, 9}


def load_env() -> dict:
    env = {}
    for line in (ROOT / ".env").read_text().splitlines():
        if "=" in line:
            k, _, v = line.partition("=")
            env[k.strip()] = v.strip()
    return env


def build_windows(path: Path) -> list[dict]:
    """Slice LoCoMo sessions into non-overlapping turn windows."""
    convs = json.loads(path.read_text())
    windows = []
    for ci, conv in enumerate(convs):
        c = conv["conversation"]
        speakers = f"{c.get('speaker_a','?')} and {c.get('speaker_b','?')}"
        for key in sorted(k for k in c if re.fullmatch(r"session_\d+", k)):
            turns = c[key]
            if not isinstance(turns, list):
                continue
            date = c.get(f"{key}_date_time", "unknown")
            for i in range(0, len(turns), WINDOW):
                chunk = [t for t in turns[i : i + WINDOW] if t.get("text")]
                if len(chunk) < 3:  # too short to hold anything durable
                    continue
                windows.append(
                    {
                        "id": f"{conv.get('sample_id', ci)}::{key}::{i}",
                        "conv_index": ci,
                        "split": "val" if ci in HELDOUT_CONVS else "train",
                        "session_date": date,
                        "speakers": speakers,
                        "turns": [
                            {
                                "dia_id": t["dia_id"],
                                "speaker": t["speaker"],
                                "text": t["text"],
                            }
                            for t in chunk
                        ],
                    }
                )
    return windows


def normalise(obj: dict, valid_ids: set[str]) -> dict | None:
    """Enforce the parts of the contract the model tends to drift on."""
    if not isinstance(obj, dict) or not isinstance(obj.get("memories"), list):
        return None
    out = []
    for m in obj["memories"]:
        if not isinstance(m, dict):
            continue
        if not all(
            m.get(k) for k in ("type", "subject", "predicate", "object", "statement")
        ):
            continue
        if m["type"] not in {"fact", "preference", "episode", "task"}:
            continue
        subj = str(m["subject"]).strip()
        pred = str(m["predicate"]).strip()
        m["entity_key"] = f"{subj}::{pred}".lower().replace(" ", "_")
        ev = [e for e in (m.get("evidence") or []) if e in valid_ids]
        if not ev:  # a memory with no traceable source violates the provenance gate
            continue
        m["evidence"] = ev
        et = m.get("event_time")
        m["event_time"] = (
            et if isinstance(et, str) and re.match(r"^\d{4}-\d{2}-\d{2}", et) else None
        )
        try:
            m["confidence"] = max(0.0, min(1.0, float(m.get("confidence", 0.5))))
        except (TypeError, ValueError):
            m["confidence"] = 0.5
        out.append(
            {
                "type": m["type"],
                "subject": subj,
                "predicate": pred,
                "object": str(m["object"]).strip(),
                "statement": str(m["statement"]).strip(),
                "entity_key": m["entity_key"],
                "event_time": m["event_time"],
                "confidence": round(m["confidence"], 2),
                "evidence": ev,
            }
        )
    return {"memories": out}


async def one(client, sem, dep, w, stats, retries=5):
    valid_ids = {t["dia_id"] for t in w["turns"]}
    user = build_user_prompt(w["session_date"], w["speakers"], w["turns"])
    async with sem:
        for attempt in range(retries):
            try:
                r = await client.chat.completions.create(
                    model=dep,
                    messages=[
                        {"role": "system", "content": SYSTEM_PROMPT},
                        {"role": "user", "content": user},
                    ],
                    response_format={
                        "type": "json_schema",
                        "json_schema": {
                            "name": "extraction",
                            "strict": True,
                            "schema": EXTRACTION_SCHEMA,
                        },
                    },
                    max_completion_tokens=2000,
                )
                raw = r.choices[0].message.content or "{}"
                parsed = normalise(json.loads(raw), valid_ids)
                if parsed is None:
                    raise ValueError("schema mismatch after parse")
                stats["ok"] += 1
                stats["mem"] += len(parsed["memories"])
                return {**w, "target": parsed}
            except Exception as e:  # noqa: BLE001 - retry everything, report at the end
                msg = str(e)
                if attempt == retries - 1:
                    stats["fail"] += 1
                    stats["last_err"] = msg[:200]
                    return None
                # 429s carry a retry hint; otherwise exponential backoff with jitter
                wait = 20 if "429" in msg or "rate" in msg.lower() else 2**attempt
                await asyncio.sleep(wait + random.random() * 2)


async def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int, default=0, help="0 = all windows")
    ap.add_argument("--concurrency", type=int, default=6)
    ap.add_argument("--out", default="data/silver.jsonl")
    args = ap.parse_args()

    env = load_env()
    client = AsyncAzureOpenAI(
        api_key=env["AZURE_OPENAI_KEY"],
        azure_endpoint=env["AZURE_OPENAI_ENDPOINT"],
        api_version=API_VERSION,
        timeout=180.0,
    )
    dep = env["AZURE_OPENAI_DEPLOYMENT"]

    windows = build_windows(ROOT / "data" / "locomo10.json")
    if args.limit:
        windows = windows[: args.limit]
    n_train = sum(1 for w in windows if w["split"] == "train")
    print(
        f"windows: {len(windows)}  (train {n_train} / val {len(windows)-n_train})",
        flush=True,
    )

    out_path = ROOT / args.out
    out_path.parent.mkdir(parents=True, exist_ok=True)
    done = set()
    if out_path.exists():  # resume
        for line in out_path.read_text().splitlines():
            try:
                done.add(json.loads(line)["id"])
            except Exception:
                pass
        print(f"resuming, {len(done)} already done", flush=True)
    todo = [w for w in windows if w["id"] not in done]

    sem = asyncio.Semaphore(args.concurrency)
    stats = {"ok": 0, "fail": 0, "mem": 0, "last_err": ""}
    t0 = time.time()
    tasks = [asyncio.create_task(one(client, sem, dep, w, stats)) for w in todo]

    with out_path.open("a") as f:
        for i, fut in enumerate(asyncio.as_completed(tasks), 1):
            res = await fut
            if res:
                f.write(json.dumps(res) + "\n")
                f.flush()
            if i % 25 == 0 or i == len(tasks):
                el = time.time() - t0
                rate = i / el * 60
                eta = (len(tasks) - i) / max(rate, 0.01)
                print(
                    f"  {i}/{len(tasks)}  ok={stats['ok']} fail={stats['fail']} "
                    f"mem={stats['mem']}  {rate:.1f}/min  eta {eta:.0f}m",
                    flush=True,
                )

    print(
        f"\ndone in {(time.time()-t0)/60:.1f}m  ok={stats['ok']} fail={stats['fail']} "
        f"memories={stats['mem']}"
    )
    if stats["fail"]:
        print(f"last error: {stats['last_err']}", file=sys.stderr)


if __name__ == "__main__":
    asyncio.run(main())
