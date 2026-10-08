"""The acceptance rules must match what the Rust engine enforces (src/knowledge.rs and friends)."""

import json
import uuid

import pytest

import rules

EVENTS = [
    {"role": "user", "content": "We use pnpm in payments-api. Deploys go out on Thursdays."},
    {"role": "tool", "content": "error: port 5433 is busy"},
]
EXISTING = [{"id": "00000000-0000-4000-8000-000000000001", "subject_id": "10000000-0000-4000-8000-000000000001"}]


def claim(**over):
    base = {
        "source_indices": [0], "quotes": ["We use pnpm in payments-api"],
        "subject": {"name": "payments-api", "entity_type": "project", "aliases": []},
        "predicate": "uses_package_manager", "value": "pnpm", "statement": "payments-api uses pnpm.",
        "cardinality": "single", "kind": "fact", "confidence": 0.9, "valid_from": None, "event_at": None,
        "entities": [], "correction": False, "explanation": "user states it", "related": [],
    }
    base.update(over)
    return base


def check(*claims):
    return rules.validate_extraction({"claims": list(claims)}, EVENTS, EXISTING)


def test_a_valid_claim_is_accepted_and_canonical_order_is_fixed():
    out = check(claim())
    assert list(out[0].keys())[:3] == ["source_indices", "quotes", "subject"]


def test_empty_claims_are_valid():
    assert check() == []


def test_quote_must_occur_verbatim_and_the_hint_names_the_right_event():
    with pytest.raises(rules.Invalid, match="does not occur verbatim in event 1; that text does occur in event 0"):
        check(claim(source_indices=[1]))
    with pytest.raises(rules.Invalid, match="copy the exact characters"):
        check(claim(quotes=["we use PNPM"]))


def test_parallel_arrays_must_match_in_length():
    with pytest.raises(rules.Invalid, match="parallel arrays"):
        check(claim(source_indices=[0, 0]))


def test_index_out_of_range_is_rejected():
    with pytest.raises(rules.Invalid, match="cites source index 5"):
        check(claim(source_indices=[5]))


def test_enums_and_ranges_are_enforced():
    with pytest.raises(rules.Invalid, match="entity_type"):
        check(claim(subject={"name": "x", "entity_type": "service", "aliases": []}))
    with pytest.raises(rules.Invalid, match="claim kind"):
        check(claim(kind="note"))
    with pytest.raises(rules.Invalid, match="unknown variant"):
        check(claim(cardinality="many"))
    with pytest.raises(rules.Invalid, match="between zero and one"):
        check(claim(confidence=1.5))
    with pytest.raises(rules.Invalid, match="nonempty"):
        check(claim(value="  "))


def test_related_must_reference_existing_facts_with_an_allowed_relation():
    rel = {"assertion_id": EXISTING[0]["id"], "relation": "contradicts", "explanation": "replaces npm"}
    assert check(claim(related=[rel]))
    with pytest.raises(rules.Invalid, match="existing facts"):
        check(claim(related=[{**rel, "assertion_id": str(uuid.uuid4())}]))
    with pytest.raises(rules.Invalid, match="not allowed"):
        check(claim(related=[{**rel, "relation": "replaces"}]))
    with pytest.raises(rules.Invalid, match="nonempty explanation"):
        check(claim(related=[{**rel, "explanation": " "}]))


def test_timestamps_accept_rfc3339_and_plain_dates_only():
    assert check(claim(valid_from="2026-10-05T09:00:00Z", event_at="2026-10-05"))
    with pytest.raises(rules.Invalid):
        check(claim(valid_from="last Thursday"))


FACTS = [
    {"id": "a0000000-0000-4000-8000-000000000001", "subject_id": "b0000000-0000-4000-8000-000000000001"},
    {"id": "a0000000-0000-4000-8000-000000000002", "subject_id": "b0000000-0000-4000-8000-000000000001"},
]


def obs(**over):
    base = {"subject_id": FACTS[0]["subject_id"], "statement": "s", "predicate": "p", "value": "v", "confidence": 0.8,
            "supports": [FACTS[0]["id"], FACTS[1]["id"]], "explanation": "e"}
    base.update(over)
    return base


def test_consolidation_needs_two_distinct_supports_from_the_given_facts():
    assert rules.validate_consolidation({"observations": [obs()]}, FACTS)
    assert rules.validate_consolidation({"observations": []}, FACTS) == []
    with pytest.raises(rules.Invalid, match="two distinct supports"):
        rules.validate_consolidation({"observations": [obs(supports=[FACTS[0]["id"]] * 2)]}, FACTS)
    with pytest.raises(rules.Invalid, match="must come from the provided facts"):
        rules.validate_consolidation({"observations": [obs(supports=[FACTS[0]["id"], str(uuid.uuid4())])]}, FACTS)
    with pytest.raises(rules.Invalid, match="subject must belong"):
        rules.validate_consolidation({"observations": [obs(subject_id=str(uuid.uuid4()))]}, FACTS)


def test_reflection_citations_must_come_from_the_evidence():
    allowed = {FACTS[0]["id"]}
    ok = {"answer": "pnpm", "citations": [FACTS[0]["id"]], "insufficient_evidence": False}
    assert rules.validate_reflect(ok, allowed)["answer"] == "pnpm"
    assert rules.validate_reflect({"answer": "?", "citations": [], "insufficient_evidence": True}, allowed)
    with pytest.raises(rules.Invalid):
        rules.validate_reflect({**ok, "citations": [str(uuid.uuid4())]}, allowed)
    with pytest.raises(rules.Invalid):
        rules.validate_reflect({**ok, "citations": []}, allowed)


def test_secret_patterns():
    assert rules.leaks_secret("key sk-live-4eC39HqLyjWDarjtT1zdp7dc")
    assert rules.leaks_secret("postgres://admin:hunter22@db.internal/app")
    assert rules.leaks_secret("-----BEGIN RSA PRIVATE KEY-----")
    assert not rules.leaks_secret("payments-api listens on port 8087")
