"""The prompts the memory engine sends to its worker model, rendered the way the Rust code does.

The engine (src/worker/prompts.rs, src/worker/window.rs, src/model.rs) is the source of truth.
This module reads the same template files and repeats the same shortening and windowing rules,
so a training example is byte for byte what the served model will be given. The shared fixture
tests/fixtures/extract_prompt_parity.json is rendered by both sides and compared in
test_prompt_parity.py and in the Rust unit tests; if either side drifts, one of them fails.
"""

from __future__ import annotations

import json
import math
import re
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
WORKER_DIR = REPO / "src" / "worker"

EXTRACT_TEMPLATE = (WORKER_DIR / "extract_prompt.txt").read_text().rstrip("\n")
CONSOLIDATE_TEMPLATE = (WORKER_DIR / "consolidate_prompt.txt").read_text().rstrip("\n")
REFLECT_TEMPLATE = (REPO / "src" / "v2" / "reflect_prompt.txt").read_text().rstrip("\n")
SYSTEM = (WORKER_DIR / "worker_system.txt").read_text().rstrip("\n")

# Constants repeated from src/worker/window.rs.
MAX_EVENT_CHARS = 1600
HEAD_CHARS = 1100
TAIL_CHARS = 360
WINDOW_EVENT_TOKENS = 2800
MAX_INPUT_CHARS = 160
BULKY_INPUT_KEYS = {
    "content", "text", "new_string", "old_string", "newText", "oldText", "edits", "diff",
}
# Constant repeated from src/worker/prompts.rs.
MAX_PROMPT_TOKENS = 6000


def dumps(value) -> str:
    """serde_json::to_string: compact, keys sorted (no preserve_order), non-ASCII kept."""
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False, sort_keys=True)


def estimate_tokens(text: str) -> int:
    """src/model.rs estimate_tokens: a token per digit, one per three other characters, +10%."""
    digits = sum(1 for c in text if "0" <= c <= "9")
    others = len(text) - digits
    raw = digits + math.ceil(others / 3)
    return raw + math.ceil(raw / 10)


def shorten_content(text: str) -> str:
    total = len(text)
    if total <= MAX_EVENT_CHARS:
        return text
    head = text[:HEAD_CHARS]
    tail = text[total - TAIL_CHARS:]
    return f"{head}\n[... {total - HEAD_CHARS - TAIL_CHARS} characters omitted ...]\n{tail}"


def _clip_end(text: str, limit: int) -> str:
    return text if len(text) <= limit else text[:limit] + "..."


def compact_metadata(metadata) -> dict:
    out: dict = {}
    if not isinstance(metadata, dict):
        return out
    for key in ("tool", "is_error", "exit_code"):
        if metadata.get(key) is not None:
            out[key] = metadata[key]
    tool_input = metadata.get("input")
    if isinstance(tool_input, dict):
        small = {}
        for key, value in tool_input.items():
            if key in BULKY_INPUT_KEYS:
                continue
            if isinstance(value, str):
                small[key] = _clip_end(value, MAX_INPUT_CHARS)
            elif isinstance(value, (bool, int, float)):
                small[key] = value
        if small:
            out["input"] = small
    return out


def compact_event(event: dict) -> dict:
    """An event as the prompt shows it. `occurred_at` must be an RFC3339 UTC string."""
    occurred = event["occurred_at"]
    # Second resolution, like the Rust side: 2026-10-07T09:30:15Z
    occurred = occurred.split(".")[0].rstrip("Z") + "Z"
    return {
        "role": event["role"],
        "content": shorten_content(event["content"]),
        "occurred_at": occurred,
        "metadata": compact_metadata(event.get("metadata")),
    }


def windows(compact_events: list[dict]) -> list[tuple[int, int]]:
    """src/worker/window.rs windows: consecutive [start, end) ranges within the token budget."""
    ranges: list[tuple[int, int]] = []
    start, used = 0, 0
    for index, event in enumerate(compact_events):
        cost = estimate_tokens(dumps(event))
        if index > start and used + cost > WINDOW_EVENT_TOKENS:
            ranges.append((start, index))
            start, used = index, 0
        used += cost
    if start < len(compact_events):
        ranges.append((start, len(compact_events)))
    return ranges


def wrap_events(compact_events: list[dict]) -> list[dict]:
    return [{"source_index": i, "event": e} for i, e in enumerate(compact_events)]


def fill(template: str, values: dict[str, str]) -> str:
    """src/worker/prompts.rs fill_template: one pass, values are never scanned for placeholders."""
    return re.sub(r"\{\{(\w+)\}\}", lambda m: values[m.group(1)], template)


def extract_prompt(existing: list[dict], wrapped_events: list[dict]) -> str:
    return fill(EXTRACT_TEMPLATE, {"EXISTING": dumps(existing), "EVENTS": dumps(wrapped_events)})


def consolidate_prompt(facts: list[dict]) -> str:
    return fill(CONSOLIDATE_TEMPLATE, {"FACTS": dumps(facts)})


def reflect_prompt(question: str, context: str) -> str:
    return fill(REFLECT_TEMPLATE, {"QUESTION": json.dumps(question, ensure_ascii=False), "EVIDENCE": context})


def fit_existing(existing: list[dict], wrapped_events: list[dict],
                 prompt_budget: int = 12288, repair_reserve: int = 1536) -> list[dict]:
    """The case src/worker/prompts.rs fit_existing_facts handles: keep the snapshot when the
    prompt fits, otherwise drop the oldest facts (the snapshot is newest first) until it does.
    The engine ranks facts that the window mentions first; training windows rarely overflow, so
    the plain rule is enough for data and the difference is not part of the parity fixture."""
    target = min(prompt_budget - min(repair_reserve, prompt_budget // 4), MAX_PROMPT_TOKENS)

    def fits(facts):
        return estimate_tokens(extract_prompt(facts, wrapped_events)) <= target

    if fits(existing):
        return existing
    low, high = 0, len(existing)
    while low < high:
        mid = (low + high + 1) // 2
        if fits(existing[:mid]):
            low = mid
        else:
            high = mid - 1
    return existing[:low]


def chat_prompt(tokenizer, user: str, enable_thinking: bool = False) -> str:
    """The text the served model sees: the Modelfile template applied to SYSTEM and the prompt.
    The student never thinks aloud, so the empty think block of Qwen3's template is part of it."""
    return tokenizer.apply_chat_template(
        [{"role": "system", "content": SYSTEM}, {"role": "user", "content": user}],
        tokenize=False,
        add_generation_prompt=True,
        enable_thinking=enable_thinking,
    )
