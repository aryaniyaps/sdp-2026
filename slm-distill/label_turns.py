"""Label turn-sized episodes, which is what the Pi extension actually retains.

The extension sends one batch per agent turn (a user prompt and what followed it until the agent
went idle). label.py treats a whole session as one episode, which gives denser windows and an
existing-facts snapshot that stays fixed for the whole session. This pass replays selected sessions
turn by turn instead: the snapshot grows after every turn, many windows hold nothing worth keeping,
and the student has to learn to answer with an empty list. All sessions of the test timelines are
used; for training, 40% of sessions are chosen by hash. The snapshot at the start of each session
comes from the whole-session pass, so both passes see the same history.

    python label_turns.py --workers 12
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import sys
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

import prompt
import rules
from build_dataset import split_of
from label import Snapshot, ask, clean
from teacher import Teacher

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
TRAIN_SHARE = 40  # percent of train/val sessions replayed by turn


def turns(raw: list[dict]) -> list[tuple[int, int]]:
    starts = [i for i, e in enumerate(raw) if e["role"] == "user"]
    if not starts or starts[0] != 0:
        starts = [0] + [s for s in starts if s != 0]
    bounds = starts + [len(raw)]
    return list(zip(bounds[:-1], bounds[1:]))


def selected(timeline_id: str, session: int) -> bool:
    if split_of(timeline_id) == "test":
        return True
    digest = hashlib.sha256(f"{timeline_id}:{session}".encode()).digest()
    return digest[0] * 100 // 256 < TRAIN_SHARE


def label_timeline(teacher: Teacher, tl: dict, whole: dict, out_dir: Path) -> str:
    path = out_dir / f"{tl['id']}.json"
    if path.exists():
        return "kept"
    state = Snapshot(tl["id"])
    records, dropped, episodes = [], 0, 0
    whole_by_session: dict[int, list[dict]] = {}
    for r in whole["records"]:
        whole_by_session.setdefault(r["session"], []).append(r)
    for si, session in enumerate(tl["sessions"]):
        raw = session["events"]
        # What the whole-session pass accepted for this session, used to advance the shared history.
        carried = []
        for r in whole_by_session.get(si, []):
            for c in r["claims"]:
                source = raw[r["range"][0] + c["source_indices"][0]]
                carried.append({**c, "_role": source["role"], "_at": source["occurred_at"]})
        if selected(tl["id"], si):
            live = copy.deepcopy(state)
            for a, b in turns(raw):
                episodes += 1
                compact = [prompt.compact_event(e) for e in raw[a:b]]
                snapshot = live.rows()
                accepted = []
                for wi, (wa, wb) in enumerate(prompt.windows(compact)):
                    wrapped = prompt.wrap_events(compact[wa:wb])
                    existing = prompt.fit_existing(snapshot, wrapped)
                    user = prompt.extract_prompt(existing, wrapped)
                    try:
                        claims, tries = ask(teacher, user, raw[a + wa:a + wb], existing, f"turn:{tl['id']}:{si}:{a}:{wi}")
                    except rules.Invalid:
                        dropped += 1
                        continue
                    claims = clean(claims)
                    for c in claims:
                        source = raw[a + wa + c["source_indices"][0]]
                        accepted.append({**c, "_role": source["role"], "_at": source["occurred_at"]})
                    records.append({
                        "timeline": tl["id"], "session": si, "episode": [a, b], "window": wi,
                        "range": [a + wa, a + wb], "existing": existing, "events": compact[wa:wb],
                        "raw_events": raw[a + wa:a + wb], "prompt": user, "claims": claims, "tries": tries,
                    })
                live.apply(accepted, raw[a]["occurred_at"])
        state.apply(carried, raw[0]["occurred_at"])
    path.write_text(json.dumps({"id": tl["id"], "records": records, "dropped_windows": dropped, "episodes": episodes},
                               ensure_ascii=False))
    return f"{len(records)} windows in {episodes} turns, {dropped} dropped"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--timelines", type=Path, default=DATA / "timelines")
    ap.add_argument("--whole", type=Path, default=DATA / "labeled")
    ap.add_argument("--out", type=Path, default=DATA / "labeled_turns")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--reverse", action="store_true")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    teacher = Teacher(DATA / "teacher_cache", DATA / "teacher_usage.jsonl")
    jobs = []
    for f in sorted(args.timelines.glob("tl*.json"), reverse=args.reverse):
        whole = args.whole / f.name
        if not whole.exists():
            print(f"skipping {f.stem}: the whole-session pass has not labeled it yet", flush=True)
            continue
        try:
            jobs.append((json.loads(f.read_text()), json.loads(whole.read_text())))
        except json.JSONDecodeError:
            print(f"skipping {f.stem}: file not complete yet", flush=True)
    failures = 0
    with ThreadPoolExecutor(args.workers) as pool:
        futures = {pool.submit(label_timeline, teacher, tl, whole, args.out): tl["id"] for tl, whole in jobs}
        for n, future in enumerate(as_completed(futures), 1):
            try:
                status = future.result()
            except Exception as err:
                failures += 1
                status = f"FAILED: {err}"
            print(f"[{n}/{len(jobs)}] {futures[future]}: {status}", flush=True)
    print(teacher.summary())
    if failures:
        sys.exit(f"{failures} timelines failed")


if __name__ == "__main__":
    main()
