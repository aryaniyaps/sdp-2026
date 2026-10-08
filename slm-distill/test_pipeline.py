"""Snapshot bookkeeping, splits, windows and the Ollama model spec."""

import json

import build_dataset
import export
import label
import label_turns
import prompt


def c(subject, predicate, value, cardinality="single", at="2026-10-05T09:00:00Z"):
    return {
        "subject": {"name": subject, "entity_type": "project", "aliases": []}, "predicate": predicate, "value": value,
        "statement": f"{subject} {predicate} {value}", "cardinality": cardinality, "kind": "fact", "confidence": 0.9,
        "valid_from": None, "event_at": None, "quotes": ["q"], "_role": "user", "_at": at,
    }


def test_a_single_valued_slot_is_replaced_and_a_multiple_one_accumulates():
    s = label.Snapshot("tl0001")
    s.apply([c("api", "uses_package_manager", "npm"), c("api", "depends_on", "redis", "multiple")], "2026-10-05T09:00:00Z")
    s.apply([c("api", "uses_package_manager", "pnpm"), c("api", "depends_on", "kafka", "multiple"),
             c("api", "depends_on", "REDIS", "multiple")], "2026-10-06T09:00:00Z")
    values = sorted((f["predicate"], f["value"]) for f in s.rows())
    assert values == [("depends_on", "kafka"), ("depends_on", "redis"), ("uses_package_manager", "pnpm")]


def test_snapshot_rows_have_the_engine_shape_and_ids_are_stable():
    a, b = label.Snapshot("tl0001"), label.Snapshot("tl0001")
    for s in (a, b):
        s.apply([c("api", "port", "8087")], "2026-10-05T09:00:00Z")
    row = a.rows()[0]
    assert set(row) == {"id", "subject", "subject_id", "predicate", "value", "cardinality", "valid_from"}
    assert row == b.rows()[0]


def test_splits_are_by_timeline_index():
    assert [build_dataset.split_of(f"tl{n:04d}") for n in (0, 259, 260, 279, 280, 319)] == \
        ["train", "train", "val", "val", "test", "test"]


def test_turns_start_at_user_events_and_cover_the_session():
    raw = [{"role": r} for r in ("user", "assistant", "tool", "user", "tool", "user", "assistant")]
    assert label_turns.turns(raw) == [(0, 3), (3, 5), (5, 7)]
    assert label_turns.turns([{"role": "assistant"}, {"role": "user"}, {"role": "tool"}]) == [(0, 1), (1, 3)]


def test_ollama_model_spec_file_is_current():
    assert json.loads(export.SPEC_FILE.read_text()) == export.spec(), "run python export.py --write-spec"


def test_modelfile_carries_the_template_and_system_text():
    text = export.modelfile()
    assert "<think>" in text and prompt.SYSTEM in text and 'PARAMETER stop "<|im_end|>"' in text
