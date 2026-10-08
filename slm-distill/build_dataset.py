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
    ap.add_argument("--aux", type=Path, default=DATA / "aux")
    ap.add_argument("--out", type=Path, default=DATA / "sets")
    ap.add_argument("--no-aux", action="store_true", help="extraction examples only")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    files = {s: (args.out / f"{s}.jsonl").open("w") for s in ("train", "val", "test")}
    counts: Counter = Counter()
    for path in sorted(args.labeled.glob("tl*.json")):
        labeled = json.loads(path.read_text())
        split = split_of(labeled["id"])
        for r in labeled["records"]:
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
            for r in aux["consolidate"] + aux["reflect"]:
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
