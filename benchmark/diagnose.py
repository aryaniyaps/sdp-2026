#!/usr/bin/env python3
"""Sorts a run's wrong answers by where they went wrong, for diagnosis on the dev split.

    python3 benchmark/diagnose.py --run locomo-dev-v2 [--condition enhanced]

For each question the primary judge marked wrong it decides, from LoCoMo's evidence turns:
  extraction  no stored assertion of the bank cites an evidence turn
  retrieval   such an assertion exists, but no recalled hit citing an evidence turn reached the context
  reader      an evidence turn was in the context, yet the answer was judged wrong
and compares with raw_hybrid and full_context on the same question. Writes runs/<run>/diagnosis.json.
"""

import argparse
import collections
import json
import subprocess

import run as bench


def cited_turns(namespace):
    """Dialogue turns cited by the bank's active assertions (via their source quotes)."""
    sql = (
        "SELECT DISTINCT e.metadata->>'dia_id' FROM assertions a JOIN assertion_sources s ON s.assertion_id=a.id "
        "JOIN chunks c ON c.id=s.chunk_id JOIN raw_events e ON e.id=c.raw_event_id "
        f"WHERE a.namespace='{namespace}' AND a.status IN ('active','contested','superseded')"
    )
    out = subprocess.check_output(["docker", "exec", "sdp-bench-postgres-1", "psql", "-U", "memory", "-d", "memory",
                                   "-tAc", sql], text=True)
    return {line.strip() for line in out.splitlines() if line.strip()}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run", required=True)
    parser.add_argument("--split", default="dev")
    parser.add_argument("--condition", default="enhanced")
    args = parser.parse_args()
    run = bench.BENCH / "runs" / args.run
    banks, items = bench.select(args.split, *bench.load())
    judge = list(bench.CONFIG["judges"])[0]
    cited = {b: cited_turns(bench.namespace(run, b)) for b in banks}
    label = lambda c, i: (lambda p: bench.read_json(p)["label"] if p.exists() else None)(
        run / "judgments" / judge / c / f"{i['id']}.json")
    rows, counts = [], collections.Counter()
    for item in items:
        if label(args.condition, item) is not False:
            continue
        evidence = set(item["evidence_turns"])
        recall = bench.read_json(run / "recall" / args.condition / f"{item['id']}.json")
        packed = {s.get("metadata", {}).get("dia_id") for h in recall["results"] for s in h["sources"]}
        if not evidence & cited[item["bank"]]:
            cause = "extraction"
        elif not evidence & packed:
            cause = "retrieval"
        else:
            cause = "reader"
        counts[(cause, item["category"])] += 1
        rows.append({
            "id": item["id"], "category": item["category"], "cause": cause, "question": item["question"],
            "gold": item["answer"],
            "answer": bench.read_json(run / "answers" / args.condition / f"{item['id']}.json")["hypothesis"],
            **{f"{c}_correct": label(c, item) for c in ("raw_hybrid", "enhanced_raw", "full_context")},
            "evidence_turns": sorted(evidence),
        })
    summary = collections.Counter(r["cause"] for r in rows)
    by_category = collections.defaultdict(dict)
    for (cause, category), n in counts.items():
        by_category[category][cause] = n
    result = {"condition": args.condition, "wrong": len(rows), "causes": dict(summary),
              "by_category": by_category, "items": rows}
    bench.atomic_json(run / "diagnosis.json", result)
    print(json.dumps({k: result[k] for k in ("condition", "wrong", "causes", "by_category")}, indent=2))


if __name__ == "__main__":
    main()
