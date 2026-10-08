"""Typed memory extraction schema.

Mirrors the write path in the sdp-2026 memory engine:
  Normalize + chunk -> Typed extraction -> Canonicalize + dedupe -> Version + conflict resolve

The student SLM is trained to emit exactly this JSON given a session window.
"""

MEMORY_TYPES = ["fact", "preference", "episode", "task"]

# JSON Schema used both for teacher structured-output and for validating student samples.
EXTRACTION_SCHEMA = {
    "type": "object",
    "additionalProperties": False,
    "required": ["memories"],
    "properties": {
        "memories": {
            "type": "array",
            "items": {
                "type": "object",
                "additionalProperties": False,
                "required": [
                    "type",
                    "subject",
                    "predicate",
                    "object",
                    "statement",
                    "entity_key",
                    "event_time",
                    "confidence",
                    "evidence",
                ],
                "properties": {
                    "type": {"type": "string", "enum": MEMORY_TYPES},
                    "subject": {"type": "string"},
                    "predicate": {"type": "string"},
                    "object": {"type": "string"},
                    "statement": {"type": "string"},
                    # canonical dedupe key: lowercase subject::predicate, spaces -> _
                    "entity_key": {"type": "string"},
                    # ISO-8601 date if the window states one, else null
                    "event_time": {"type": ["string", "null"]},
                    "confidence": {"type": "number"},
                    # dia_id list of the turns that support this memory
                    "evidence": {"type": "array", "items": {"type": "string"}},
                },
            },
        }
    },
}

SYSTEM_PROMPT = """You extract durable, atomic memories from a window of a conversation.

Emit ONLY memories that remain useful after the conversation ends. Skip greetings, \
small talk, acknowledgements, and anything that is purely about the current turn.

Types:
  fact       - a stable property of a person, place or thing ("Mel has two daughters")
  preference - a like, dislike, or stated choice ("Caroline prefers hiking to the gym")
  episode    - a specific dated thing that happened ("Mel visited Barcelona in May 2023")
  task       - an intention or commitment, not yet done ("Caroline plans to adopt a dog")

Rules:
- One atomic claim per memory. Never join two claims with "and".
- subject/predicate/object must be short and canonical. Resolve pronouns to the
  speaker or addressee name.
- entity_key = lowercase "subject::predicate" with spaces replaced by underscores.
- event_time: ISO-8601 (YYYY-MM-DD) only if the window or the session date supports
  it; otherwise null. Never invent a date.
- evidence: the dia_id values of the turns that support the memory.
- confidence: 0.0-1.0, your calibrated belief the memory is correct and durable.
- If the window contains nothing durable, return {"memories": []}.

Return JSON matching the provided schema. No prose, no markdown fences."""


def build_user_prompt(session_date: str, speakers: str, turns: list[dict]) -> str:
    lines = [f"[{t['dia_id']}] {t['speaker']}: {t['text']}" for t in turns]
    body = "\n".join(lines)
    return (
        f"Session date: {session_date}\n"
        f"Speakers: {speakers}\n"
        f"--- window ---\n{body}\n--- end window ---\n\n"
        "Extract the durable memories."
    )
