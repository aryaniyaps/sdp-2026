"""The training prompt must equal the prompt the Rust engine serves."""

import json

import make_parity_fixture
import prompt


def test_fixture_is_up_to_date():
    on_disk = json.loads(make_parity_fixture.OUT.read_text())
    assert on_disk == make_parity_fixture.build(), (
        "tests/fixtures/extract_prompt_parity.json is stale: run python make_parity_fixture.py"
    )


def test_prompt_fills_both_placeholders_and_has_no_trailing_newline():
    text = prompt.extract_prompt([], prompt.wrap_events([]))
    assert "{{" not in text and not text.endswith("\n")
    assert text.endswith("Existing facts: []\nEvents: []")


def test_windows_split_the_long_session_and_cover_it():
    fixture = json.loads(make_parity_fixture.OUT.read_text())["compaction"][0]
    ranges = fixture["windows"]
    assert len(ranges) > 1 and ranges[0][0] == 0
    assert all(a[1] == b[0] for a, b in zip(ranges, ranges[1:]))
    assert ranges[-1][1] == len(fixture["raw"])


def test_shortening_keeps_head_tail_and_names_the_gap():
    text = "a" * 1100 + "#" * 5000 + "z" * 360
    shown = prompt.shorten_content(text)
    assert shown.startswith("a" * 1100) and shown.endswith("z" * 360)
    assert "[... 5000 characters omitted ...]" in shown and "#" not in shown
