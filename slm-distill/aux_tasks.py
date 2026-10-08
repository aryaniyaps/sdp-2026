"""Training data for the two other jobs the worker model does: consolidation and reflection.

The engine asks its worker model three things: extract claims from events (label.py),
consolidate several facts of one subject into an observation (src/worker/consolidation.rs), and
answer a question from recalled evidence with citations (src/v2/reflect.rs). One student serves
all three, so it is trained on all three. The prompts are the engine's own template files
(src/worker/consolidate_prompt.txt, src/v2/reflect_prompt.txt), checked against the Rust code by the
shared fixture.

    python aux_tasks.py --workers 12
"""

from __future__ import annotations

import argparse
import json
import random
import sys
import uuid
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

import prompt
import rules
from label import NAMESPACE, Snapshot
from teacher import Teacher

HERE = Path(__file__).resolve().parent
DATA = HERE / "data"
REPO = HERE.parent

CONSOLIDATE_REPAIR = (
    "Return complete corrected JSON. Copy support and subject UUIDs exactly from the supplied facts; use two "
    "distinct supports with at least one belonging to the observation subject. Preserve supported observations "
    "and omit only unsupported inferences."
)


consolidate_prompt = prompt.consolidate_prompt
reflect_prompt = prompt.reflect_prompt


def recall_block(fact: dict, session_id: str, status: str = "active") -> str:
    """One hit as src/v2/recall.rs packs it into the context string."""
    valid_from = f'Some({fact["valid_from"]})'
    return (
        f'[{fact["id"]}] {fact["_statement"]} [{fact["_kind"]}; {status}; valid_from={valid_from}; '
        f"valid_to=None; event_at=None]\n"
        f'  Evidence ({session_id}, {fact["_role"]}, {fact["_at"]}): {fact["_quote"][:1200]}\n'
    )


def facts_for_consolidation(facts: list[dict], touched: set[str]) -> list[dict]:
    """The rows src/worker/consolidation.rs selects: active facts of the subjects the episode touched."""
    rows = [
        {"id": f["id"], "subject_id": f["subject_id"], "subject": f["subject"], "statement": f["_statement"],
         "kind": f["_kind"], "valid_from": f["valid_from"]}
        for f in facts if f["subject_id"] in touched
    ]
    return rows[:100]


def run_repair(teacher: Teacher, base: str, check, repair_suffix: str, tag: str, attempts: int = 3):
    text = base
    last = ""
    for repair in range(attempts):
        output = teacher.complete_json(prompt.SYSTEM, text, effort="low", tag=tag)
        try:
            return check(output), output
        except rules.Invalid as err:
            last = str(err)
            text = (f"{base}\nRepair attempt {repair}. Previous response: {prompt.dumps(output)}\n"
                    f"Validation error: {err}. {repair_suffix}")
    raise rules.Invalid(last)


QUESTION_SYSTEM = "You write test questions for a memory system. Return one JSON object only."
QUESTION_PROMPT = """These are the facts a developer's coding agent remembers about one project (id, statement, and whether a newer value replaced an older one):
{facts}

Write {n} questions the developer might ask the agent later, in the developer's own voice, short and natural. Mix:
- direct: answered by exactly one fact above
- combine: needs two or three facts together
- changed: about a value that was replaced, where only the newest value is right
- unanswerable: a plausible question about this project that none of the facts answers

Return {{"questions":[{{"q":"...","kind":"direct|combine|changed|unanswerable","needs":[fact ids that answer it, empty for unanswerable]}}]}}. Use only ids listed above."""


def build_consolidation(teacher: Teacher, tl: dict, labeled: dict) -> list[dict]:
    state = Snapshot(tl["id"])
    out = []
    by_session: dict[int, list[dict]] = {}
    for r in labeled["records"]:
        by_session.setdefault(r["session"], []).append(r)
    for si, session in enumerate(tl["sessions"]):
        raw = session["events"]
        accepted, touched = [], set()
        for r in by_session.get(si, []):
            for c in r["claims"]:
                source = raw[r["range"][0] + c["source_indices"][0]]
                accepted.append({**c, "_role": source["role"], "_at": source["occurred_at"]})
        state.apply(accepted, raw[0]["occurred_at"])
        for c in accepted:
            touched.add(str(uuid.uuid5(NAMESPACE, f'{tl["id"]}:entity:{c["subject"]["name"].strip().lower()}')))
        facts = facts_for_consolidation(state.detailed(), touched)
        if len(facts) < 2:
            continue
        user = consolidate_prompt(facts)
        try:
            observations, _ = run_repair(
                teacher, user, lambda o: rules.validate_consolidation(o, facts), CONSOLIDATE_REPAIR,
                f"consolidate:{tl['id']}:{si}")
        except rules.Invalid:
            continue
        out.append({"task": "consolidate", "timeline": tl["id"], "session": si, "prompt": user,
                    "target": {"observations": observations}})
    return out


def build_reflect(teacher: Teacher, tl: dict, labeled: dict, rng: random.Random) -> list[dict]:
    state = Snapshot(tl["id"])
    out = []
    by_session: dict[int, list[dict]] = {}
    for r in labeled["records"]:
        by_session.setdefault(r["session"], []).append(r)
    superseded: dict[tuple, str] = {}
    for si, session in enumerate(tl["sessions"]):
        raw = session["events"]
        accepted = []
        for r in by_session.get(si, []):
            for c in r["claims"]:
                source = raw[r["range"][0] + c["source_indices"][0]]
                accepted.append({**c, "_role": source["role"], "_at": source["occurred_at"]})
        for c in accepted:
            key = (c["subject"]["name"].strip().lower(), c["predicate"].strip().lower())
            for f in state.facts.values():
                if (f["subject"].strip().lower(), f["predicate"].strip().lower()) == key and c["cardinality"] == "single":
                    superseded[key] = f["value"]
        state.apply(accepted, raw[0]["occurred_at"])
    facts = state.detailed()
    if len(facts) < 4:
        return out
    pool = facts[:40]
    listing = "\n".join(
        f'- {f["id"]}: {f["_statement"]}' + (
            f' (replaced the older value "{superseded[(f["subject"].strip().lower(), f["predicate"].strip().lower())]}")'
            if (f["subject"].strip().lower(), f["predicate"].strip().lower()) in superseded else "")
        for f in pool)
    asked = teacher.complete_json(QUESTION_SYSTEM, QUESTION_PROMPT.format(facts=listing, n=8), effort="low",
                                  tag=f"questions:{tl['id']}")
    by_id = {f["id"]: f for f in pool}
    session_id = "pi-" + tl["id"]
    for qi, item in enumerate(asked.get("questions", [])):
        if not isinstance(item, dict) or not isinstance(item.get("q"), str):
            continue
        needs = [i for i in item.get("needs", []) if i in by_id]
        kind = item.get("kind")
        if kind != "unanswerable" and not needs:
            continue
        distractors = [f for f in pool if f["id"] not in needs]
        rng.shuffle(distractors)
        chosen = [by_id[i] for i in needs] + distractors[: rng.randint(2, 5) if kind != "unanswerable" else rng.randint(3, 6)]
        rng.shuffle(chosen)
        context = "".join(recall_block(f, session_id) for f in chosen)
        user = reflect_prompt(item["q"], context)
        allowed = {f["id"] for f in chosen}
        try:
            answer, _ = run_repair(
                teacher, user, lambda o: rules.validate_reflect(o, allowed),
                "Return complete corrected JSON citing only IDs that appear in the evidence.",
                f"reflect:{tl['id']}:{qi}")
        except rules.Invalid:
            continue
        if answer["insufficient_evidence"] != (kind == "unanswerable"):
            continue
        out.append({"task": "reflect", "timeline": tl["id"], "kind": kind, "prompt": user, "target": answer,
                    "allowed": sorted(allowed), "needs": needs})
    return out


def process(teacher: Teacher, tl_path: Path, labeled_dir: Path, out_dir: Path) -> str:
    out = out_dir / tl_path.name
    if out.exists():
        return "kept"
    tl = json.loads(tl_path.read_text())
    lab_path = labeled_dir / tl_path.name
    if not lab_path.exists():
        return "no labels"
    labeled = json.loads(lab_path.read_text())
    rng = random.Random(int(tl["id"][2:]) * 31 + 5)
    consolidate = build_consolidation(teacher, tl, labeled)
    reflect = build_reflect(teacher, tl, labeled, rng)
    out.write_text(json.dumps({"id": tl["id"], "consolidate": consolidate, "reflect": reflect}, ensure_ascii=False))
    return f"{len(consolidate)} consolidate, {len(reflect)} reflect"


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--timelines", type=Path, default=DATA / "timelines")
    ap.add_argument("--labeled", type=Path, default=DATA / "labeled")
    ap.add_argument("--out", type=Path, default=DATA / "aux")
    ap.add_argument("--workers", type=int, default=8)
    ap.add_argument("--limit", type=int)
    args = ap.parse_args()
    args.out.mkdir(parents=True, exist_ok=True)
    teacher = Teacher(DATA / "teacher_cache", DATA / "teacher_usage.jsonl")
    files = sorted(args.timelines.glob("tl*.json"))[: args.limit]
    failures = 0
    with ThreadPoolExecutor(args.workers) as pool:
        futures = {pool.submit(process, teacher, f, args.labeled, args.out): f for f in files}
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
