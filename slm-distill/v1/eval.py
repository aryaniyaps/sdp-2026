"""Evaluate the student against the teacher's held-out labels.

Runs the SAME 142 held-out windows through the base model zero-shot and through
the LoRA-tuned student, so the delta isolates what distillation bought. Reports
agreement with the teacher, JSON validity, and latency.

Note on what this measures: the teacher's output is the reference, so these are
AGREEMENT scores, not correctness against human ground truth. A student that
perfectly imitates a wrong teacher scores 1.0 here. That is the standard framing
for distillation, and it is what the Review-1 gate is about, but it should be
stated plainly rather than presented as accuracy.

  python eval.py --adapter out/qwen3-1.7b-memex-lora/final --limit 142
"""

import argparse
import json
import time
from pathlib import Path

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig

from schema import SYSTEM_PROMPT, build_user_prompt
from train_lora import BASE

ROOT = Path(__file__).parent


def load_val(path: Path, limit: int):
    rows = [json.loads(l) for l in path.read_text().splitlines()]
    rows = [r for r in rows if r.get("split") == "val"]
    return rows[:limit] if limit else rows


def keyset(mems):
    """A memory is 'the same' if it asserts the same typed relation about the
    same canonical subject. Object wording is allowed to differ."""
    return {(m.get("type"), m.get("entity_key")) for m in mems if m.get("entity_key")}


def prf(pred, gold):
    tp = len(pred & gold)
    p = tp / len(pred) if pred else (1.0 if not gold else 0.0)
    r = tp / len(gold) if gold else 1.0
    f = 2 * p * r / (p + r) if p + r else 0.0
    return p, r, f


def parse(raw):
    raw = raw.strip()
    if raw.startswith("```"):
        raw = raw.split("```")[1].lstrip("json").strip()
    try:
        o = json.loads(raw)
        return o.get("memories", []) if isinstance(o, dict) else None
    except Exception:
        return None


@torch.no_grad()
def run(model, tok, rows, tag, max_new=768, dump=None):
    dump = [] if dump is None else dump
    agg = {"p": 0.0, "r": 0.0, "f": 0.0, "valid": 0, "lat": [], "npred": 0, "ngold": 0}
    for i, r in enumerate(rows, 1):
        prompt = tok.apply_chat_template(
            [
                {"role": "system", "content": SYSTEM_PROMPT},
                {
                    "role": "user",
                    "content": build_user_prompt(
                        r["session_date"], r["speakers"], r["turns"]
                    ),
                },
            ],
            tokenize=False,
            add_generation_prompt=True,
            enable_thinking=False,
        )
        ids = tok(prompt, return_tensors="pt").to(model.device)
        torch.cuda.synchronize()
        t0 = time.time()
        out = model.generate(
            **ids,
            max_new_tokens=max_new,
            do_sample=False,
            pad_token_id=tok.pad_token_id,
        )
        torch.cuda.synchronize()
        agg["lat"].append(time.time() - t0)
        text = tok.decode(out[0][ids["input_ids"].shape[1] :], skip_special_tokens=True)

        mems = parse(text)
        dump.append(
            {"id": r["id"], "raw": text, "pred": mems, "gold": r["target"]["memories"]}
        )
        gold = keyset(r["target"]["memories"])
        agg["ngold"] += len(gold)
        if mems is None:
            continue  # invalid JSON scores zero on all three
        agg["valid"] += 1
        agg["npred"] += len(mems)
        p, rc, f = prf(keyset(mems), gold)
        agg["p"] += p
        agg["r"] += rc
        agg["f"] += f
        if i % 25 == 0:
            print(
                f"    [{tag}] {i}/{len(rows)}  running F1={agg['f']/i:.3f}", flush=True
            )

    Path(f"data/gen_{tag}.json").write_text(json.dumps(dump, indent=2))
    n = len(rows)
    lat = sorted(agg["lat"])
    return {
        "tag": tag,
        "n": n,
        "precision": agg["p"] / n,
        "recall": agg["r"] / n,
        "f1": agg["f"] / n,
        "json_valid_pct": 100 * agg["valid"] / n,
        "p50_ms": 1000 * lat[len(lat) // 2],
        "p95_ms": 1000 * lat[int(len(lat) * 0.95)],
        "mem_per_window_pred": agg["npred"] / max(agg["valid"], 1),
        "mem_per_window_gold": agg["ngold"] / n,
    }


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--adapter", default="out/qwen3-1.7b-memex-lora/final")
    ap.add_argument("--limit", type=int, default=0)
    ap.add_argument("--out", default="data/eval.json")
    args = ap.parse_args()

    rows = load_val(ROOT / "data" / "silver.jsonl", args.limit)
    print(f"held-out windows: {len(rows)}")

    tok = AutoTokenizer.from_pretrained(BASE)
    if tok.pad_token is None:
        tok.pad_token = tok.eos_token
    qcfg = BitsAndBytesConfig(
        load_in_4bit=True,
        bnb_4bit_quant_type="nf4",
        bnb_4bit_use_double_quant=True,
        bnb_4bit_compute_dtype=torch.bfloat16,
    )

    results = []
    print("\n=== A. base model, zero-shot (no distillation) ===")
    m = AutoModelForCausalLM.from_pretrained(
        BASE, quantization_config=qcfg, dtype=torch.bfloat16, device_map={"": 0}
    )
    m.eval()
    results.append(run(m, tok, rows, "base-zeroshot"))
    del m
    torch.cuda.empty_cache()

    ad = ROOT / args.adapter
    if ad.exists():
        print("\n=== B. distilled student (LoRA) ===")
        from peft import PeftModel

        m = AutoModelForCausalLM.from_pretrained(
            BASE, quantization_config=qcfg, dtype=torch.bfloat16, device_map={"": 0}
        )
        m = PeftModel.from_pretrained(m, str(ad))
        m.eval()
        results.append(run(m, tok, rows, "distilled"))
        del m
        torch.cuda.empty_cache()
    else:
        print(f"\n(no adapter at {ad} - skipping the distilled run)")

    (ROOT / args.out).write_text(json.dumps(results, indent=2))
    print("\n" + "=" * 76)
    hdr = f"{'variant':<16}{'F1':>7}{'prec':>7}{'rec':>7}{'JSON%':>8}{'p50ms':>9}{'p95ms':>9}{'mem/win':>9}"
    print(hdr)
    print("-" * 76)
    for r in results:
        print(
            f"{r['tag']:<16}{r['f1']:>7.3f}{r['precision']:>7.3f}{r['recall']:>7.3f}"
            f"{r['json_valid_pct']:>8.1f}{r['p50_ms']:>9.0f}{r['p95_ms']:>9.0f}"
            f"{r['mem_per_window_pred']:>9.1f}"
        )
    print(
        f"\nteacher averaged {results[0]['mem_per_window_gold']:.1f} memories per window"
    )
    if len(results) == 2:
        d = results[1]["f1"] - results[0]["f1"]
        print(f"distillation delta: {d:+.3f} F1")


if __name__ == "__main__":
    main()
