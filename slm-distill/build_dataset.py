"""Assemble the training, validation and test files from labeled windows and auxiliary tasks.

Splits are by timeline, never by window, so no project the student is tested on was seen in
training: timelines 0-259 train, 260-279 validation, 280-319 test.

    python build_dataset.py
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path

import prompt

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
TRAIN_MAX, VAL_MAX = 260, 280


def split_of(timeline_id: str) -> str:
    n = int(timeline_id[2:])
    return "train" if n < TRAIN_MAX else "val" if n < VAL_MAX else "test"


def compact(value) -> str:
    """The student's output text: canonical key order, no spaces, non-ASCII kept."""
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--labeled", type=Path, default=DATA / "labeled")
    ap.add_argument("--turns", type=Path, default=DATA / "labeled_turns")
    ap.add_argument("--aux", type=Path, default=DATA / "aux")
    ap.add_argument("--out", type=Path, default=DATA / "sets")
    ap.add_argument("--no-aux", action="store_true", help="extraction examples only")
    ap.add_argument("--reflect-per-timeline", type=int, default=4,
                    help="most reflection examples kept per training timeline (they are short and numerous)")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    files = {s: (args.out / f"{s}.jsonl").open("w") for s in ("train", "val", "test")}
    counts: Counter = Counter()
    for path in sorted(args.labeled.glob("tl*.json")):
        labeled = json.loads(path.read_text())
        split = split_of(labeled["id"])
        by_turn = args.turns / path.name
        turn_records = json.loads(by_turn.read_text())["records"] if by_turn.exists() else []
        turn_sessions = {r["session"] for r in turn_records}
        # Turn-sized episodes are what the extension retains. A session replayed by turn replaces its
        # whole-session windows; the test split uses turn-sized windows only.
        records = turn_records if split == "test" else \
            turn_records + [r for r in labeled["records"] if r["session"] not in turn_sessions]
        for r in records:
            row = {"task": "extract", "timeline": r["timeline"], "prompt": r["prompt"],
                   "target": compact({"claims": r["claims"]})}
            if split == "test":
                row["meta"] = {k: r[k] for k in ("session", "window", "range", "existing", "raw_events")}
            files[split].write(json.dumps(row, ensure_ascii=False) + "\n")
            counts[(split, "extract")] += 1
    if not args.no_aux:
        for path in sorted(args.aux.glob("tl*.json")):
            aux = json.loads(path.read_text())
            split = split_of(aux["id"])
            reflect = aux["reflect"] if split == "test" else aux["reflect"][: args.reflect_per_timeline]
            for r in aux["consolidate"] + reflect:
                row = {"task": r["task"], "timeline": r["timeline"], "prompt": r["prompt"], "target": compact(r["target"])}
                if r["task"] == "reflect":
                    row["meta"] = {"allowed": r["allowed"], "kind": r["kind"]}
                files[split].write(json.dumps(row, ensure_ascii=False) + "\n")
                counts[(split, r["task"])] += 1
    for f in files.values():
        f.close()
    for key in sorted(counts):
        print(f"{key[0]:5} {key[1]:12} {counts[key]}")


if __name__ == "__main__":
    main()
