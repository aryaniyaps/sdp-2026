"""The engine's acceptance rules for worker output, ported to Python.

A training label is only kept when the Rust engine would accept it, so these mirror
src/knowledge.rs (ClaimInput, validate_claim), src/worker/consolidation.rs
(validate_consolidation) and src/v2/reflect.rs (the citation checks). `canonical_claim` fixes the
key order the student is trained to write.
"""

from __future__ import annotations

import math
import re
import uuid
from datetime import datetime

ENTITY_TYPES = ["person", "organization", "project", "place", "technology", "other"]
CARDINALITIES = ["single", "multiple", "event"]
KINDS = ["fact", "preference", "profile", "goal", "episode", "procedure", "other"]
RELATIONS = ["extends", "contradicts", "causes"]


class Invalid(Exception):
    """Carries the message the engine would send back to the model in a repair prompt."""


def clip(text: str, limit: int) -> str:
    return text if len(text) <= limit else text[:limit] + "..."


def _timestamp(value, field):
    if value is None:
        return None
    if not isinstance(value, str):
        raise Invalid(f"extraction schema: {field} must be a string or null")
    try:
        datetime.fromisoformat(value.replace("Z", "+00:00")) if "T" in value else datetime.strptime(value, "%Y-%m-%d")
    except ValueError as err:
        raise Invalid(f"extraction schema: {field} is not RFC3339: {value}") from err
    return value


def _entity(obj, where):
    if not isinstance(obj, dict) or not isinstance(obj.get("name"), str) or not isinstance(obj.get("entity_type"), str):
        raise Invalid(f"extraction schema: {where} needs string name and entity_type")
    aliases = obj.get("aliases", [])
    if not isinstance(aliases, list) or not all(isinstance(a, str) for a in aliases):
        raise Invalid(f"extraction schema: {where}.aliases must be a list of strings")
    return {"name": obj["name"], "entity_type": obj["entity_type"], "aliases": aliases}


def canonical_claim(claim: dict) -> dict:
    """Types checked like serde would, defaults filled, keys in the order the student writes."""
    if not isinstance(claim, dict):
        raise Invalid("extraction schema: a claim must be an object")
    for key in ("predicate", "value", "statement", "cardinality", "kind"):
        if not isinstance(claim.get(key), str):
            raise Invalid(f"extraction schema: missing field `{key}`")
    if claim["cardinality"] not in CARDINALITIES:
        raise Invalid(f"extraction schema: unknown variant `{claim['cardinality']}`, expected one of single, multiple, event")
    confidence = claim.get("confidence")
    if isinstance(confidence, bool) or not isinstance(confidence, (int, float)):
        raise Invalid("extraction schema: missing field `confidence`")
    indices, quotes = claim.get("source_indices"), claim.get("quotes")
    if not isinstance(indices, list) or not all(isinstance(i, int) and not isinstance(i, bool) and i >= 0 for i in indices):
        raise Invalid("extraction schema: source_indices must be a list of non-negative integers")
    if not isinstance(quotes, list) or not all(isinstance(q, str) for q in quotes):
        raise Invalid("extraction schema: quotes must be a list of strings")
    related = []
    for rel in claim.get("related", []) or []:
        if not isinstance(rel, dict):
            raise Invalid("extraction schema: related entries must be objects")
        try:
            rid = str(uuid.UUID(str(rel.get("assertion_id"))))
        except ValueError as err:
            raise Invalid("extraction schema: related.assertion_id must be a UUID") from err
        related.append({"assertion_id": rid, "relation": rel.get("relation"), "explanation": rel.get("explanation", "")})
    correction = claim.get("correction", False)
    if not isinstance(correction, bool):
        raise Invalid("extraction schema: correction must be a boolean")
    return {
        "source_indices": indices,
        "quotes": quotes,
        "subject": _entity(claim.get("subject"), "subject"),
        "predicate": claim["predicate"],
        "value": claim["value"],
        "statement": claim["statement"],
        "cardinality": claim["cardinality"],
        "kind": claim["kind"],
        "confidence": confidence,
        "valid_from": _timestamp(claim.get("valid_from"), "valid_from"),
        "event_at": _timestamp(claim.get("event_at"), "event_at"),
        "entities": [_entity(e, "entities[]") for e in claim.get("entities", []) or []],
        "correction": correction,
        "explanation": claim.get("explanation", "") if isinstance(claim.get("explanation", ""), str) else "",
        "related": related,
    }


def validate_claim(c: dict, events: list[dict]) -> None:
    """src/knowledge.rs validate_claim. `events` are the full, unshortened window events."""
    for field, value in [
        ("subject.name", c["subject"]["name"]),
        ("subject.entity_type", c["subject"]["entity_type"]),
        ("predicate", c["predicate"]),
        ("value", c["value"]),
        ("statement", c["statement"]),
    ]:
        if not value.strip():
            raise Invalid(f'claim fields must be nonempty: {field} is empty in claim "{clip(c["statement"], 100)}"')
    conf = c["confidence"]
    if not math.isfinite(conf) or not (0.0 <= conf <= 1.0):
        raise Invalid(f'confidence must be finite and between zero and one, got {conf} in claim "{clip(c["statement"], 100)}"')
    for entity in [c["subject"]] + c["entities"]:
        if not entity["name"].strip():
            raise Invalid(f'an entity in claim "{clip(c["statement"], 100)}" has an empty name')
        if entity["entity_type"] not in ENTITY_TYPES:
            raise Invalid(
                f'entity "{clip(entity["name"], 60)}" has entity_type "{clip(entity["entity_type"], 40)}", '
                f"which is not allowed; use exactly one of {', '.join(ENTITY_TYPES)}"
            )
    for rel in c["related"]:
        if rel["relation"] not in RELATIONS:
            raise Invalid(f'related relation "{clip(str(rel["relation"]), 40)}" is not allowed; use exactly one of extends, contradicts, causes')
        if not str(rel["explanation"]).strip():
            raise Invalid("every related edge needs a nonempty explanation")
    if c["kind"] not in KINDS:
        raise Invalid(f'claim kind "{clip(c["kind"], 40)}" is not allowed; use exactly one of {", ".join(KINDS)}')
    if not c["source_indices"] or len(c["source_indices"]) != len(c["quotes"]):
        raise Invalid(
            f'every claim needs paired source indices and exact quotes: claim "{c["statement"]}" has '
            f'{len(c["source_indices"])} source_indices and {len(c["quotes"])} quotes; they are parallel arrays of '
            "equal length, so repeat an event index once per quote"
        )
    for i, q in zip(c["source_indices"], c["quotes"]):
        if not q.strip():
            raise Invalid(f'claim "{clip(c["statement"], 100)}" has an empty quote; every quote must be exact text copied from its event')
        if i >= len(events):
            raise Invalid(f'claim "{clip(c["statement"], 100)}" cites source index {i}, but there are {len(events)} events numbered from 0')
        if q not in events[i]["content"]:
            where = next((j for j, e in enumerate(events) if q in e["content"]), None)
            hint = (
                f"that text does occur in event {where}, so use source index {where}"
                if where is not None
                else "copy the exact characters from the event content, including punctuation, case and whitespace, or quote a shorter piece of it"
            )
            raise Invalid(
                f'claim "{clip(c["statement"], 100)}": quote "{clip(q, 120)}" does not occur verbatim in event {i}; {hint}'
            )


def validate_extraction(obj, events: list[dict], existing: list[dict]) -> list[dict]:
    """Returns the canonical claims or raises Invalid with the engine's message."""
    if not isinstance(obj, dict) or not isinstance(obj.get("claims"), list):
        raise Invalid("extraction schema: missing field `claims`")
    claims = [canonical_claim(c) for c in obj["claims"]]
    known = {f["id"] for f in existing}
    for claim in claims:
        validate_claim(claim, events)
        if any(rel["assertion_id"] not in known for rel in claim["related"]):
            raise Invalid("related assertion must come from the provided existing facts")
    return claims


def validate_consolidation(obj, facts: list[dict]) -> list[dict]:
    if not isinstance(obj, dict) or not isinstance(obj.get("observations"), list):
        raise Invalid("consolidation schema: missing field `observations`")
    out = []
    ids = {f["id"] for f in facts}
    for o in obj["observations"]:
        try:
            subject_id = str(uuid.UUID(str(o["subject_id"])))
            supports = [str(uuid.UUID(str(s))) for s in o["supports"]]
            statement, predicate, value = o["statement"], o["predicate"], o["value"]
            confidence = o["confidence"]
            explanation = o.get("explanation", "")
        except (KeyError, ValueError, TypeError) as err:
            raise Invalid(f"consolidation schema: {err}") from err
        if (
            len(set(supports)) < 2
            or isinstance(confidence, bool)
            or not isinstance(confidence, (int, float))
            or not math.isfinite(confidence)
            or not (0.0 <= confidence <= 1.0)
            or any(not isinstance(t, str) or not t.strip() for t in (statement, predicate, value))
        ):
            raise Invalid("observation requires two distinct supports, valid confidence and nonempty fields")
        if any(s not in ids for s in set(supports)):
            raise Invalid("observation support must come from the provided facts")
        if not any(f["subject_id"] == subject_id and f["id"] in set(supports) for f in facts):
            raise Invalid("observation subject must belong to at least one supplied support")
        out.append({
            "subject_id": subject_id, "statement": statement, "predicate": predicate, "value": value,
            "confidence": confidence, "supports": supports, "explanation": explanation if isinstance(explanation, str) else "",
        })
    return out


def validate_reflect(obj, allowed_ids: set[str]) -> dict:
    """src/v2/reflect.rs: citations must come from the evidence, and a real answer cites something."""
    if not isinstance(obj, dict):
        raise Invalid("reflection must be an object")
    try:
        citations = [str(uuid.UUID(str(c))) for c in obj["citations"]]
    except (KeyError, ValueError, TypeError) as err:
        raise Invalid(f"reflection citations: {err}") from err
    insufficient = obj.get("insufficient_evidence")
    if (
        any(c not in allowed_ids for c in citations)
        or not isinstance(obj.get("answer"), str)
        or not isinstance(insufficient, bool)
        or (insufficient is False and not citations)
    ):
        raise Invalid("reflection has invalid or missing citations")
    return {"answer": obj["answer"], "citations": citations, "insufficient_evidence": insufficient}


SECRET_PATTERNS = [
    re.compile(p)
    for p in (
        r"sk-[A-Za-z0-9_\-]{16,}",
        r"AKIA[0-9A-Z]{16}",
        r"ghp_[A-Za-z0-9]{20,}",
        r"xox[abp]-[A-Za-z0-9\-]{10,}",
        r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
        r"eyJ[A-Za-z0-9_\-]{10,}\.[A-Za-z0-9_\-]{10,}\.",
        r"[A-Za-z]+://[^\s:@/]+:[^\s@/]{4,}@",
    )
]


def leaks_secret(text: str) -> bool:
    return any(p.search(text) for p in SECRET_PATTERNS)
