"""Label windows of the synthetic sessions with the teacher, the way the engine would run them.

Each session is one episode. The engine builds its existing-facts snapshot when the episode
starts, cuts the events into windows, and asks the model about each window with the same
snapshot. So here the snapshot comes from the claims accepted in earlier sessions, every window
of a session shares it, and the claims of a session are applied after the session. A label is
kept only if the engine's own acceptance rules (rules.py) pass, after the same repair loop the
engine runs. The record keeps everything evaluation needs to replay the window.

    python label.py --workers 12
"""

from __future__ import annotations

import argparse
import json
import sys
import uuid
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

import prompt
import rules
from teacher import Teacher

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
NAMESPACE = uuid.UUID("6f1e1a9c-6a54-4d3a-9c53-0d0a5b1c2f77")
MAX_CLAIMS = 12
SNAPSHOT_LIMIT = 80  # src/worker/extraction.rs: LIMIT 80, newest first


def slot(subject: str, predicate: str) -> tuple[str, str]:
    return subject.strip().lower(), predicate.strip().lower()


class Snapshot:
    """The active facts of a namespace, as the engine's extraction query returns them."""

    def __init__(self, timeline_id: str):
        self.tid = timeline_id
        self.facts: dict[str, dict] = {}
        self.counter = 0

    def _id(self, kind: str, key: str) -> str:
        return str(uuid.uuid5(NAMESPACE, f"{self.tid}:{kind}:{key}"))

    def rows(self) -> list[dict]:
        ordered = sorted(self.facts.values(), key=lambda f: (f["valid_from"], f["id"]), reverse=True)
        return [{k: v for k, v in f.items() if not k.startswith("_")} for f in ordered[:SNAPSHOT_LIMIT]]

    def detailed(self) -> list[dict]:
        """Every active fact with its statement, kind and first quote, newest first."""
        return sorted(self.facts.values(), key=lambda f: (f["valid_from"], f["id"]), reverse=True)

    def apply(self, claims: list[dict], session_time: str) -> None:
        for c in claims:
            key = slot(c["subject"]["name"], c["predicate"])
            valid_from = c["valid_from"] or c["event_at"] or session_time
            if c["cardinality"] == "single":
                for fid in [fid for fid, f in self.facts.items() if slot(f["subject"], f["predicate"]) == key]:
                    del self.facts[fid]
            elif any(slot(f["subject"], f["predicate"]) == key and f["value"].lower() == c["value"].lower()
                     for f in self.facts.values()):
                continue
            self.counter += 1
            fid = self._id("fact", str(self.counter))
            self.facts[fid] = {
                "id": fid, "subject": c["subject"]["name"],
                "subject_id": self._id("entity", c["subject"]["name"].strip().lower()),
                "predicate": c["predicate"], "value": c["value"], "cardinality": c["cardinality"],
                "valid_from": valid_from if "T" in valid_from else valid_from + "T00:00:00Z",
                "_statement": c["statement"], "_kind": c["kind"],
                "_quote": c["quotes"][0], "_role": c.get("_role", "user"), "_at": c.get("_at", session_time),
            }


def clean(claims: list[dict]) -> list[dict]:
    """Drop claims that repeat a slot and value, or that carry something that looks like a secret."""
    seen, out = set(), []
    for c in claims:
        text = " ".join([c["statement"], c["value"], *c["quotes"]])
        if rules.leaks_secret(text):
            continue
        key = (*slot(c["subject"]["name"], c["predicate"]), c["value"].strip().lower())
        if key in seen:
            continue
        seen.add(key)
        out.append(c)
    return out[:MAX_CLAIMS]


def ask(teacher: Teacher, user: str, raw_events: list[dict], existing: list[dict], tag: str):
    """The engine's repair loop: up to three tries, each repair carries the last reply and error."""
    base = user
    attempt_input = user
    last_error = ""
    for repair in range(3):
        output = teacher.complete_json(prompt.SYSTEM, attempt_input, effort="low", tag=tag)
        try:
            return rules.validate_extraction(output, raw_events, existing), repair + 1
        except rules.Invalid as err:
            last_error = str(err)
            attempt_input = (
                f"{base}\nRepair attempt {repair}. Previous response: {prompt.dumps(output)}\n"
                f"Validation error: {err}. Return the complete corrected JSON, preserving all valid claims "
                "and correcting attribution/quotes from the original events."
            )
    raise rules.Invalid(last_error)


def label_timeline(teacher: Teacher, tl: dict, out_dir: Path) -> str:
    path = out_dir / f"{tl['id']}.json"
    if path.exists():
        return "kept"
    state = Snapshot(tl["id"])
    records, dropped = [], 0
    for si, session in enumerate(tl["sessions"]):
        raw = session["events"]
        compact = [prompt.compact_event(e) for e in raw]
        snapshot = state.rows()
        accepted: list[dict] = []
        for wi, (a, b) in enumerate(prompt.windows(compact)):
            wrapped = prompt.wrap_events(compact[a:b])
            existing = prompt.fit_existing(snapshot, wrapped)
            user = prompt.extract_prompt(existing, wrapped)
            try:
                claims, tries = ask(teacher, user, raw[a:b], existing, f"label:{tl['id']}:{si}:{wi}")
            except rules.Invalid:
                dropped += 1
                continue
            claims = clean(claims)
            for c in claims:
                source = raw[a + c["source_indices"][0]]
                accepted.append({**c, "_role": source["role"], "_at": source["occurred_at"]})
            records.append({
                "timeline": tl["id"], "session": si, "window": wi, "range": [a, b],
                "existing": existing, "events": compact[a:b], "raw_events": raw[a:b],
                "prompt": user, "claims": claims, "tries": tries,
            })
        state.apply(accepted, raw[0]["occurred_at"])
    path.write_text(json.dumps({"id": tl["id"], "records": records, "dropped_windows": dropped}, ensure_ascii=False))
    return f"{len(records)} windows labeled, {dropped} dropped"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--timelines", type=Path, default=DATA / "timelines")
    ap.add_argument("--out", type=Path, default=DATA / "labeled")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--limit", type=int)
    ap.add_argument("--reverse", action="store_true", help="start from the last timeline, to share a run with another process")
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    teacher = Teacher(DATA / "teacher_cache", DATA / "teacher_usage.jsonl")
    files, loaded = [], {}
    for f in sorted(args.timelines.glob("tl*.json"), reverse=args.reverse):
        try:
            loaded[f] = json.loads(f.read_text())  # a file still being written is skipped this run
        except json.JSONDecodeError:
            print(f"skipping {f.name}: not complete yet", flush=True)
            continue
        files.append(f)
    files = files[: args.limit]
    failures = 0
    with ThreadPoolExecutor(args.workers) as pool:
        futures = {pool.submit(label_timeline, teacher, loaded[f], args.out): f for f in files}
        for n, future in enumerate(as_completed(futures), 1):
            try:
                status = future.result()
            except Exception as err:
                failures += 1
                status = f"FAILED: {err}"
            print(f"[{n}/{len(files)}] {futures[future].stem}: {status}", flush=True)
    print(teacher.summary())
    if failures:
        sys.exit(f"{failures} timelines failed")


if __name__ == "__main__":
    main()
