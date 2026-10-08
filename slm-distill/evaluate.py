"""Score a served model on the held-out test timelines, through Ollama like the engine calls it.

    python evaluate.py --model memex-extractor --label student
    python evaluate.py --model qwen3-1.7b-base  --label base
    python evaluate.py --teacher-labels          --label teacher

For every test window the model gets the exact prompt the engine would send, and the engine's
repair loop (three tries, each repair carrying the previous reply and the validation error).
What is reported:

  accepted        the engine would store the window's claims (valid after at most 3 tries)
  first_try       valid with no repair
  recall          planted facts covered by a claim, among planted facts whose text was visible
  precision       claims judged supported
  trivial, unsupported, secret, injected   claim verdict rates (judge: the teacher model)
  agreement_f1    claims matched one to one with the teacher's claims by embedding similarity
  latency         wall seconds per window including repairs

The judge sees the events as the model saw them and the planted list written with the transcript,
so recall is measured against what the sessions say, not against the teacher's own output.
"""

from __future__ import annotations

import argparse
import json
import math
import statistics
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import judge
import ollama_client
import prompt
import rules
from teacher import Teacher

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
EMBED_BASE = "http://127.0.0.1:11434"  # the stack's CPU Ollama, holds qwen3-embedding:0.6b
EMBED_MODEL = "qwen3-embedding:0.6b"
MATCH = 0.80


def percentile(values: list[float], q: float) -> float:
    if not values:
        return float("nan")
    ordered = sorted(values)
    return ordered[min(len(ordered) - 1, math.ceil(q * len(ordered)) - 1)]


def run_extract(base: str, model: str, row: dict) -> dict:
    """The engine's loop for one window. Returns the accepted claims or the last error."""
    meta = row["meta"]
    base_prompt = row["prompt"]
    attempt_input = base_prompt
    wall = tokens = 0
    last_error, tries = "", 0
    for repair in range(3):
        tries += 1
        response = ollama_client.generate(base, model, attempt_input)
        wall += response["wall_seconds"]
        tokens += response.get("eval_count", 0)
        text = response.get("response", "")
        try:
            output = json.loads(text)
            claims = rules.validate_extraction(output, meta["raw_events"], meta["existing"])
            return {"accepted": True, "tries": tries, "claims": claims, "wall": wall, "tokens": tokens}
        except (json.JSONDecodeError, rules.Invalid) as err:
            last_error = str(err)
            attempt_input = (f"{base_prompt}\nRepair attempt {repair}. Previous response: {text}\n"
                             f"Validation error: {err}. Return the complete corrected JSON, preserving all valid claims "
                             "and correcting attribution/quotes from the original events.")
    return {"accepted": False, "tries": tries, "claims": [], "error": last_error, "wall": wall, "tokens": tokens}


def f1_by_embedding(pairs: list[tuple[list[dict], list[dict]]]) -> dict:
    """Greedy one to one matching of claim statements by cosine similarity over all windows."""
    tp = fp = fn = 0
    flags: list[tuple[bool, bool]] = []  # (student correction flag, teacher correction flag) of matched claims
    texts = [c["statement"] for got, ref in pairs for c in got + ref]
    if not texts:
        return {"precision": float("nan"), "recall": float("nan"), "f1": float("nan"), "matched_claims": 0}
    vectors = []
    for i in range(0, len(texts), 64):
        vectors += ollama_client.embed(EMBED_BASE, EMBED_MODEL, texts[i:i + 64])
    cursor = 0
    for got, ref in pairs:
        gv, rv = vectors[cursor:cursor + len(got)], vectors[cursor + len(got):cursor + len(got) + len(ref)]
        cursor += len(got) + len(ref)

        def cos(a, b):
            return sum(x * y for x, y in zip(a, b)) / (math.sqrt(sum(x * x for x in a)) * math.sqrt(sum(y * y for y in b)))

        scored = sorted(((cos(a, b), i, j) for i, a in enumerate(gv) for j, b in enumerate(rv)), reverse=True)
        used_g, used_r, matched = set(), set(), 0
        for score, i, j in scored:
            if score < MATCH:
                break
            if i in used_g or j in used_r:
                continue
            used_g.add(i)
            used_r.add(j)
            matched += 1
            flags.append((bool(got[i]["correction"]), bool(ref[j]["correction"])))
        tp += matched
        fp += len(got) - matched
        fn += len(ref) - matched
    precision = tp / (tp + fp) if tp + fp else 0.0
    recall = tp / (tp + fn) if tp + fn else 0.0
    teacher_flagged = [g for g in flags if g[1]]
    return {
        "precision": precision, "recall": recall,
        "f1": 2 * precision * recall / (precision + recall) if precision + recall else 0.0,
        "matched_claims": len(flags),
        # A wrong correction flag turns an update into a contradiction (src/knowledge_store/assertions.rs).
        "correction_flag_agreement": sum(a == b for a, b in flags) / len(flags) if flags else float("nan"),
        "teacher_corrections_found": sum(a for a, _ in teacher_flagged) / len(teacher_flagged) if teacher_flagged else float("nan"),
        "false_corrections": sum(1 for a, b in flags if a and not b),
    }


def planted_for(window: dict, timelines: dict) -> list[dict]:
    meta = window["meta"]
    tl = timelines[window["timeline"]]
    lo, hi = meta["range"]
    return [p for p in tl["planted"] if p["session"] == meta["session"] and lo <= p["event"] < hi]


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--model")
    ap.add_argument("--ollama", default="http://127.0.0.1:11436")
    ap.add_argument("--teacher-labels", action="store_true", help="score the teacher's own labels instead of a model")
    ap.add_argument("--label", required=True)
    ap.add_argument("--sets", type=Path, default=DATA / "sets")
    ap.add_argument("--limit", type=int)
    ap.add_argument("--workers", type=int, default=2, help="parallel requests to Ollama")
    ap.add_argument("--skip-judge", action="store_true")
    ap.add_argument("--tasks", default="extract,consolidate,reflect")
    args = ap.parse_args()
    if bool(args.model) == args.teacher_labels:
        sys.exit("give exactly one of --model or --teacher-labels")
    tasks = set(args.tasks.split(","))
    rows = [json.loads(line) for line in (args.sets / "test.jsonl").read_text().splitlines()]
    timelines = {p.stem: json.loads(p.read_text()) for p in (DATA / "timelines").glob("tl*.json")}
    teacher = None if args.skip_judge and not args.teacher_labels else Teacher(DATA / "teacher_cache", DATA / "teacher_usage.jsonl")
    report: dict = {"label": args.label, "model": args.model or "teacher labels"}
    (DATA / "eval").mkdir(exist_ok=True)

    # ------------------------------------------------------------------ extraction
    if "extract" in tasks:
        windows = [r for r in rows if r["task"] == "extract"][: args.limit]
        if args.teacher_labels:
            results = [{"accepted": True, "tries": 1, "claims": json.loads(r["target"])["claims"], "wall": 0.0, "tokens": 0} for r in windows]
        else:
            with ThreadPoolExecutor(args.workers) as pool:
                results = list(pool.map(lambda r: run_extract(args.ollama, args.model, r), windows))
        accepted = [r for r in results if r["accepted"]]
        ext = {
            "windows": len(windows),
            "accepted": len(accepted) / len(windows),
            "first_try": sum(1 for r in results if r["accepted"] and r["tries"] == 1) / len(windows),
            "claims_per_window": statistics.mean(len(r["claims"]) for r in accepted) if accepted else 0,
        }
        if not args.teacher_labels:
            walls = [r["wall"] for r in results]
            ext["latency_p50_s"] = percentile(walls, 0.5)
            ext["latency_p95_s"] = percentile(walls, 0.95)
            ext["tokens_per_s"] = sum(r["tokens"] for r in results) / max(1e-9, sum(r["wall"] for r in results))
            ext["failures"] = [r["error"][:160] for r in results if not r["accepted"]][:5]
        if not args.teacher_labels:
            ref = [json.loads(r["target"])["claims"] for r in windows]
            ext["agreement"] = f1_by_embedding([(r["claims"], t) for r, t in zip(results, ref)])
        if not args.skip_judge:
            def grade(item):
                window, result = item
                if not result["accepted"]:
                    return None
                planted = planted_for(window, timelines)
                shown = window["meta"]["raw_events"]
                compact = [prompt.compact_event(e) for e in shown]
                if not planted and not result["claims"]:
                    return {"planted": [], "claims": []}
                return judge.grade_window(teacher, compact, planted, result["claims"],
                                          f"judge:{args.label}:{window['timeline']}:{window['meta']['session']}:{window['meta']['window']}",
                                          existing=window["meta"]["existing"])
            with ThreadPoolExecutor(8) as pool:
                graded = list(pool.map(grade, zip(windows, results)))
            visible = covered = nclaims = 0
            verdicts: dict[str, int] = {}
            for g in graded:
                if not g:
                    continue
                for p in g.get("planted", []):
                    if p.get("visible"):
                        visible += 1
                        covered += 1 if p.get("covered") else 0
                for c in g.get("claims", []):
                    nclaims += 1
                    verdicts[c.get("verdict", "?")] = verdicts.get(c.get("verdict", "?"), 0) + 1
            ext["judge"] = {
                "planted_visible": visible, "recall": covered / visible if visible else float("nan"),
                "claims": nclaims, **{k: v / nclaims for k, v in sorted(verdicts.items())},
            }
            # Regex backstop for credentials, independent of the judge
            ext["secret_regex_hits"] = sum(1 for r in results for c in r["claims"]
                                           if rules.leaks_secret(" ".join([c["statement"], c["value"], *c["quotes"]])))
        report["extract"] = ext

    # --------------------------------------------------------------- consolidation
    if "consolidate" in tasks:
        items = [r for r in rows if r["task"] == "consolidate"][: args.limit]
        outcome = []
        for r in items:
            if args.teacher_labels:
                outcome.append({"accepted": True, "empty": not json.loads(r["target"])["observations"], "wall": 0.0})
                continue
            facts = json.loads(r["prompt"].split("Facts: ", 1)[1])
            attempt_input, ok, wall = r["prompt"], False, 0.0
            for repair in range(3):
                resp = ollama_client.generate(args.ollama, args.model, attempt_input)
                wall += resp["wall_seconds"]
                try:
                    obs = rules.validate_consolidation(json.loads(resp["response"]), facts)
                    ok = True
                    break
                except (json.JSONDecodeError, rules.Invalid) as err:
                    attempt_input = (f'{r["prompt"]}\nRepair attempt {repair}. Previous response: {resp["response"]}\n'
                                     f"Validation error: {err}. {__import__('aux_tasks').CONSOLIDATE_REPAIR}")
            outcome.append({"accepted": ok, "empty": ok and not obs, "wall": wall})
        teacher_empty = [not json.loads(r["target"])["observations"] for r in items]
        report["consolidate"] = {
            "items": len(items),
            "accepted": sum(o["accepted"] for o in outcome) / max(1, len(items)),
            "empty_agrees_with_teacher": sum(o["empty"] == t for o, t in zip(outcome, teacher_empty)) / max(1, len(items)),
        }

    # ------------------------------------------------------------------- reflection
    if "reflect" in tasks:
        items = [r for r in rows if r["task"] == "reflect"][: args.limit]
        rec = []
        for r in items:
            ref = json.loads(r["target"])
            if args.teacher_labels:
                rec.append({"accepted": True, "insufficient_ok": True, "correct": True})
                continue
            allowed = set(r["meta"]["allowed"])
            attempt_input, ok, answer = r["prompt"], False, None
            for repair in range(3):
                resp = ollama_client.generate(args.ollama, args.model, attempt_input)
                try:
                    answer = rules.validate_reflect(json.loads(resp["response"]), allowed)
                    ok = True
                    break
                except (json.JSONDecodeError, rules.Invalid) as err:
                    attempt_input = (f'{r["prompt"]}\nRepair attempt {repair}. Previous response: {resp["response"]}\n'
                                     f"Validation error: {err}. Return complete corrected JSON citing only IDs that appear in the evidence.")
            entry = {"accepted": ok, "insufficient_ok": ok and answer["insufficient_evidence"] == ref["insufficient_evidence"], "correct": None}
            if ok and not args.skip_judge and not ref["insufficient_evidence"] and entry["insufficient_ok"]:
                question = json.loads(r["prompt"].split("Question: ", 1)[1].split("\nEvidence:\n", 1)[0])
                context = r["prompt"].split("\nEvidence:\n", 1)[1]
                entry["correct"] = bool(judge.grade_reply(teacher, question, context, ref["answer"], answer["answer"],
                                                         f"judge-reply:{args.label}:{r['timeline']}").get("correct"))
            rec.append(entry)
        answerable = [e for e in rec if e["correct"] is not None]
        report["reflect"] = {
            "items": len(items),
            "accepted": sum(e["accepted"] for e in rec) / max(1, len(items)),
            "insufficient_flag_matches_teacher": sum(e["insufficient_ok"] for e in rec) / max(1, len(items)),
            "answer_correct_when_answerable": (sum(e["correct"] for e in answerable) / len(answerable)) if answerable else None,
        }

    out = DATA / "eval" / f"{args.label}.json"
    out.write_text(json.dumps(report, indent=1))
    print(json.dumps(report, indent=1))
    if teacher:
        print(teacher.summary())


if __name__ == "__main__":
    main()
