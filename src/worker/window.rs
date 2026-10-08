//! Shrink retained evidence to what a small model reads, and cut it into windows that fit.
//!
//! A coding session can hold hundreds of events and a single tool result can be 100 kB, far
//! more than a 1.7B student reads well. Each event is shortened for the prompt only. Claims are
//! still validated against the full event content, so a quote taken from the part that is shown
//! is accepted. `slm-distill/prompt.py` repeats these rules for training data and is checked
//! against this file by a shared fixture.

use crate::{knowledge::EvidenceEvent, model::estimate_tokens};
use serde_json::{Map, Value, json};
use std::ops::Range;

/// Longest event content shown to the model, in characters. A longer one keeps its start and its
/// end, with a marker between them that names how much was left out.
pub const MAX_EVENT_CHARS: usize = 1600;
const HEAD_CHARS: usize = 1100;
const TAIL_CHARS: usize = 360;
/// Estimated tokens of events in one window.
pub const WINDOW_EVENT_TOKENS: usize = 2800;
/// Longest string kept from a tool call's input, in characters.
const MAX_INPUT_CHARS: usize = 160;
/// Tool input fields that carry whole files or patches. They are never shown.
const BULKY_INPUT_KEYS: [&str; 8] = [
    "content",
    "text",
    "new_string",
    "old_string",
    "newText",
    "oldText",
    "edits",
    "diff",
];

fn clip_end(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("...");
    out
}

/// The part of an event's content that the model sees.
pub fn shorten_content(text: &str) -> String {
    let total = text.chars().count();
    if total <= MAX_EVENT_CHARS {
        return text.to_string();
    }
    let head: String = text.chars().take(HEAD_CHARS).collect();
    let tail: String = text.chars().skip(total - TAIL_CHARS).collect();
    format!(
        "{head}\n[... {} characters omitted ...]\n{tail}",
        total - HEAD_CHARS - TAIL_CHARS
    )
}

/// The metadata the model sees: which tool ran, whether it failed, its exit code and its short
/// scalar inputs (a command, a path). Identifiers, commit ids and file bodies are dropped.
fn compact_metadata(metadata: &Value) -> Value {
    let mut out = Map::new();
    let Some(object) = metadata.as_object() else {
        return Value::Object(out);
    };
    for key in ["tool", "is_error", "exit_code"] {
        if let Some(value) = object.get(key).filter(|value| !value.is_null()) {
            out.insert(key.to_string(), value.clone());
        }
    }
    if let Some(input) = object.get("input").and_then(Value::as_object) {
        let mut small = Map::new();
        for (key, value) in input {
            if BULKY_INPUT_KEYS.contains(&key.as_str()) {
                continue;
            }
            match value {
                Value::String(text) => {
                    small.insert(key.clone(), json!(clip_end(text, MAX_INPUT_CHARS)));
                }
                Value::Number(_) | Value::Bool(_) => {
                    small.insert(key.clone(), value.clone());
                }
                _ => {}
            }
        }
        if !small.is_empty() {
            out.insert("input".to_string(), Value::Object(small));
        }
    }
    Value::Object(out)
}

/// One event as the prompt shows it.
pub fn compact_event(event: &EvidenceEvent) -> Value {
    json!({
        "role": event.role,
        "content": shorten_content(&event.content),
        "occurred_at": event.occurred_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        "metadata": compact_metadata(&event.metadata),
    })
}

/// Cut compact events into consecutive ranges whose estimated size stays within
/// `WINDOW_EVENT_TOKENS`. A range always holds at least one event.
pub fn windows(events: &[Value]) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let (mut start, mut used) = (0, 0);
    for (index, event) in events.iter().enumerate() {
        let cost = estimate_tokens(&event.to_string());
        if index > start && used + cost > WINDOW_EVENT_TOKENS {
            ranges.push(start..index);
            start = index;
            used = 0;
        }
        used += cost;
    }
    if start < events.len() {
        ranges.push(start..events.len());
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn event(role: &str, content: &str, metadata: Value) -> EvidenceEvent {
        EvidenceEvent {
            role: role.into(),
            content: content.into(),
            occurred_at: Utc.with_ymd_and_hms(2026, 10, 7, 9, 30, 15).unwrap(),
            metadata,
        }
    }

    #[test]
    fn short_content_is_untouched() {
        assert_eq!(shorten_content("use pnpm"), "use pnpm");
        let exact = "x".repeat(MAX_EVENT_CHARS);
        assert_eq!(shorten_content(&exact), exact);
    }

    #[test]
    fn long_content_keeps_head_and_tail_and_names_the_gap() {
        let text = format!(
            "{}{}{}",
            "a".repeat(1100),
            "#".repeat(5000),
            "z".repeat(360)
        );
        let shown = shorten_content(&text);
        assert!(shown.starts_with(&"a".repeat(1100)));
        assert!(shown.ends_with(&"z".repeat(360)));
        assert!(shown.contains("[... 5000 characters omitted ...]"));
        assert!(!shown.contains('#'));
    }

    #[test]
    fn multibyte_content_is_cut_on_characters() {
        let text = "é".repeat(MAX_EVENT_CHARS + 500);
        let shown = shorten_content(&text);
        assert!(shown.starts_with(&"é".repeat(1100)) && shown.ends_with(&"é".repeat(360)));
        assert!(shown.contains("[... 640 characters omitted ...]"));
    }

    #[test]
    fn a_quote_from_the_shown_part_is_still_in_the_full_content() {
        let full = format!("error: port 5433 is busy\n{}", "log line\n".repeat(900));
        assert!(shorten_content(&full).contains("port 5433 is busy"));
        assert!(full.contains("port 5433 is busy"));
    }

    #[test]
    fn metadata_keeps_tool_facts_and_drops_identifiers_and_bodies() {
        let metadata = json!({
            "tool_call_id":"call_1","commit":"abc123","entry_id":"e1","truncated":false,
            "tool":"edit","is_error":false,"exit_code":0,
            "input":{"path":"src/main.rs","newText":"fn main(){}","edits":[{"a":1}],"count":3,"flag":true,"note":"x".repeat(400)}
        });
        let compact = compact_metadata(&metadata);
        assert_eq!(compact["tool"], "edit");
        assert_eq!(compact["exit_code"], 0);
        assert_eq!(compact["is_error"], false);
        assert_eq!(compact["input"]["path"], "src/main.rs");
        assert_eq!(compact["input"]["count"], 3);
        assert_eq!(compact["input"]["flag"], true);
        assert_eq!(
            compact["input"]["note"].as_str().unwrap().chars().count(),
            163
        );
        for dropped in ["tool_call_id", "commit", "entry_id", "truncated"] {
            assert!(compact.get(dropped).is_none(), "{dropped}");
        }
        for dropped in ["newText", "edits"] {
            assert!(compact["input"].get(dropped).is_none(), "{dropped}");
        }
    }

    #[test]
    fn missing_or_odd_metadata_gives_an_empty_object() {
        assert_eq!(compact_metadata(&Value::Null), json!({}));
        assert_eq!(compact_metadata(&json!("text")), json!({}));
        assert_eq!(compact_metadata(&json!({"tool":null})), json!({}));
    }

    #[test]
    fn compact_event_has_second_resolution_time_and_sorted_keys() {
        let compact = compact_event(&event("user", "hi", json!({"commit":"c"})));
        assert_eq!(
            compact.to_string(),
            r#"{"content":"hi","metadata":{},"occurred_at":"2026-10-07T09:30:15Z","role":"user"}"#
        );
    }

    fn sized(chars: usize) -> Value {
        json!({"content":"w".repeat(chars)})
    }

    #[test]
    fn windows_cover_every_event_once_and_in_order() {
        let events: Vec<Value> = (0..40).map(|_| sized(900)).collect();
        let ranges = windows(&events);
        assert!(ranges.len() > 1);
        assert_eq!(ranges[0].start, 0);
        assert_eq!(ranges.last().unwrap().end, events.len());
        for pair in ranges.windows(2) {
            assert_eq!(pair[0].end, pair[1].start);
        }
        for range in &ranges {
            let tokens: usize = events[range.clone()]
                .iter()
                .map(|event| estimate_tokens(&event.to_string()))
                .sum();
            assert!(
                range.len() == 1 || tokens <= WINDOW_EVENT_TOKENS,
                "{range:?} {tokens}"
            );
        }
    }

    #[test]
    fn one_oversized_event_gets_its_own_window() {
        let events = vec![sized(10), sized(20_000), sized(10)];
        assert_eq!(windows(&events), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn small_sessions_are_one_window_and_empty_ones_none() {
        let events: Vec<Value> = (0..5).map(|_| sized(100)).collect();
        assert_eq!(windows(&events), vec![0..5]);
        assert!(windows(&[]).is_empty());
    }

    #[test]
    fn compaction_and_windows_match_the_fixture_shared_with_the_training_code() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/extract_prompt_parity.json"
        ))
        .unwrap();
        for case in fixture["compaction"].as_array().unwrap() {
            let events: Vec<EvidenceEvent> = serde_json::from_value(case["raw"].clone()).unwrap();
            let compact: Vec<Value> = events.iter().map(compact_event).collect();
            let expected: Vec<&str> = case["compact_json"]
                .as_array()
                .unwrap()
                .iter()
                .map(|line| line.as_str().unwrap())
                .collect();
            let got: Vec<String> = compact.iter().map(|event| event.to_string()).collect();
            assert_eq!(got, expected, "case {}", case["name"]);
            let ranges: Vec<[usize; 2]> = windows(&compact)
                .iter()
                .map(|range| [range.start, range.end])
                .collect();
            let want: Vec<[usize; 2]> = serde_json::from_value(case["windows"].clone()).unwrap();
            assert_eq!(ranges, want, "case {}", case["name"]);
        }
    }
}
