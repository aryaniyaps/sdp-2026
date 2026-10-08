"""Write tests/fixtures/extract_prompt_parity.json, the file the Rust unit tests and
test_prompt_parity.py both check. Run it after changing src/worker/extract_prompt.txt or the
shortening and windowing rules, then run `cargo test --lib` and `pytest slm-distill`."""

import json
from pathlib import Path

import prompt

OUT = prompt.REPO / "tests" / "fixtures" / "extract_prompt_parity.json"


def raw(role, content, at, metadata=None):
    return {"role": role, "content": content, "occurred_at": at, "metadata": metadata or {}}


def build() -> dict:
    long_log = "error: port 5433 is busy\n" + "".join(f"log line {i} with é and 日本\n" for i in range(160))
    session = [
        raw("user", "We use pnpm in payments-api. Deploys happen on Thursdays.", "2026-10-05T09:00:00Z", {"commit": "abc"}),
        raw("assistant", "Noted: pnpm for payments-api.", "2026-10-05T09:00:05Z", {"entry_id": "e1"}),
        raw("tool", long_log, "2026-10-05T09:00:20Z",
            {"tool": "bash", "is_error": True, "exit_code": 1, "tool_call_id": "c1",
             "input": {"command": "pnpm test " + "x" * 300, "cwd": "/work/payments-api", "timeout": 30, "env": {"A": "b"}}}),
        raw("tool", "ok", "2026-10-05T09:00:21Z",
            {"tool": "write", "is_error": False, "input": {"path": "src/a.ts", "content": "BIG FILE BODY", "newText": "x"}}),
    ] + [raw("user", "step " + "word " * 400, f"2026-10-05T09:{i:02d}:00Z") for i in range(1, 9)]
    compact = [prompt.compact_event(e) for e in session]
    existing = [{
        "id": "00000000-0000-4000-8000-000000000001", "subject": "payments-api",
        "subject_id": "10000000-0000-4000-8000-000000000001", "predicate": "uses_package_manager",
        "value": "npm", "cardinality": "single", "valid_from": "2026-09-01T00:00:00Z",
    }]
    small = [prompt.compact_event(e) for e in session[:2]]
    cases = []
    for name, facts, events in [
        ("no existing facts", [], small),
        ("one existing fact, quotes and unicode", existing,
         [prompt.compact_event(raw("user", 'He said "use é and 日本" \\ and a tab\there', "2026-10-05T09:00:00Z"))]),
        ("empty events", [], []),
    ]:
        cases.append({
            "name": name, "existing": facts, "events": events,
            "prompt": prompt.extract_prompt(facts, prompt.wrap_events(events)),
        })
    compaction = [{
        "name": "session with long log, secrets-free metadata and a window split",
        "raw": session,
        "compact_json": [prompt.dumps(e) for e in compact],
        "windows": [list(r) for r in prompt.windows(compact)],
    }]
    facts = [{
        "id": "00000000-0000-4000-8000-000000000001", "subject_id": "10000000-0000-4000-8000-000000000001",
        "subject": "payments-api", "statement": 'payments-api uses "pnpm" {{EVENTS}}', "kind": "fact",
        "valid_from": "2026-10-05T09:00:00Z",
    }]
    consolidate = [{"name": "one fact with placeholder look-alike text", "facts": facts,
                    "prompt": prompt.consolidate_prompt(facts)}]
    context = (
        "[00000000-0000-4000-8000-000000000001] payments-api uses pnpm [fact; active; "
        "valid_from=Some(2026-10-05T09:00:00Z); valid_to=None; event_at=None]\n"
        "  Evidence (pi-1, user, 2026-10-05T09:00:00Z): We use pnpm in payments-api.\n"
    )
    reflect = [{"name": "question with quotes and unicode", "question": 'Which package manager does "payments-api" use? é {{EVIDENCE}}',
                "context": context, "prompt": prompt.reflect_prompt('Which package manager does "payments-api" use? é {{EVIDENCE}}', context)}]
    return {"cases": cases, "compaction": compaction, "consolidate": consolidate, "reflect": reflect}


if __name__ == "__main__":
    OUT.write_text(json.dumps(build(), indent=1, ensure_ascii=False) + "\n")
    print("wrote", OUT)
