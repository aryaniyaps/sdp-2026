//! Constrain worker output at decoding time; domain validators remain authoritative.
//! RawValue preserves schema property order without changing canonical JSON elsewhere.
use crate::{AppError, worker::fill_template};
use serde_json::{Value, json, value::RawValue};

pub const WIRE_INSTRUCTION: &str = include_str!("paired_sources.txt");

fn invalid(message: &str) -> AppError {
    AppError::Provider(format!("worker output schema: {message}"))
}
fn first_json(text: &str) -> Result<(Value, &str), AppError> {
    let mut stream = serde_json::Deserializer::from_str(text).into_iter::<Value>();
    let value = stream
        .next()
        .ok_or_else(|| invalid("missing prompt data"))?
        .map_err(|e| invalid(&e.to_string()))?;
    Ok((value, &text[stream.byte_offset()..]))
}
fn object(properties: Value) -> Value {
    let required: Vec<_> = properties.as_object().unwrap().keys().cloned().collect();
    json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
fn strings(values: Vec<Value>) -> Value {
    if values.is_empty() {
        json!({"type":"string"})
    } else {
        json!({"enum":values})
    }
}
fn quote_candidates(content: &str) -> Vec<&str> {
    // Split the synthetic compaction marker before sentence detection. Otherwise
    // the ellipsis followed by whitespace creates a candidate ending in "[...",
    // which is visible in the prompt but never existed in the original event.
    if let Some((head, rest)) = content.split_once("\n[... ")
        && let Some((count, tail)) = rest.split_once(" characters omitted ...]\n")
        && count.parse::<usize>().is_ok()
    {
        let mut candidates = quote_candidates(head);
        for quote in quote_candidates(tail) {
            if !candidates.contains(&quote) {
                candidates.push(quote);
            }
        }
        return candidates;
    }
    let mut candidates = vec![content];
    candidates.extend(content.lines());
    // Sentence boundaries retain exact source bytes, including punctuation.
    let mut start = 0;
    let mut previous = ' ';
    for (index, ch) in content.char_indices() {
        if ch.is_whitespace() && matches!(previous, '.' | '!' | '?') {
            candidates.push(&content[start..index]);
            start = index;
        }
        previous = ch;
    }
    candidates.push(content[start..].trim_start());
    let mut unique = Vec::new();
    for text in candidates {
        let text = text.trim_start();
        if !text.trim().is_empty()
            && !text.contains("characters omitted")
            && !unique.contains(&text)
        {
            unique.push(text);
        }
    }
    unique
}

#[cfg(test)]
mod quote_compaction_tests {
    #[test]
    fn every_quote_candidate_survives_in_the_uncompacted_source() {
        let original = format!(
            "Pass product IDs to the API. {} End of retained source.",
            "abc ".repeat(1500)
        );
        let compact = crate::worker::shorten_content(&original);
        let candidates = super::quote_candidates(&compact);
        assert!(!candidates.is_empty());
        for quote in candidates {
            assert!(
                original.contains(quote),
                "synthetic quote candidate: {quote:?}"
            );
        }
    }
}

/// Only parse markers in the trusted template. JSON is consumed before looking for the next
/// marker, so event text containing `Events:` or template placeholders cannot replace data.
pub fn prepare(prompt: &str) -> Result<(String, Box<RawValue>, bool), AppError> {
    let mut paired = false;
    let schema = if prompt.starts_with("You are the extraction worker") {
        let (_, tail) = prompt
            .split_once("\nExisting facts: ")
            .ok_or_else(|| invalid("missing existing facts"))?;
        let (existing, tail) = first_json(tail)?;
        let tail = tail
            .trim_start()
            .strip_prefix("Events: ")
            .ok_or_else(|| invalid("missing events"))?;
        let (events, _) = first_json(tail)?;
        let events = events
            .as_array()
            .ok_or_else(|| invalid("events must be an array"))?;
        let mut branches = Vec::new();
        for entry in events {
            let index = entry["source_index"]
                .as_u64()
                .ok_or_else(|| invalid("missing source index"))?;
            let content = entry["event"]["content"]
                .as_str()
                .ok_or_else(|| invalid("missing event content"))?;
            let quotes = quote_candidates(content);
            if !quotes.is_empty() {
                // source_index first, quote second: match the model's learned evidence order.
                branches.push(format!(r#"{{"type":"object","properties":{{"source_index":{{"const":{index}}},"quote":{{"enum":{}}}}},"required":["source_index","quote"],"additionalProperties":false}}"#, serde_json::to_string(&quotes).unwrap()));
            }
        }
        let ids = existing
            .as_array()
            .ok_or_else(|| invalid("facts must be an array"))?
            .iter()
            .filter_map(|f| f.get("id").cloned())
            .collect::<Vec<_>>();
        let related = if ids.is_empty() {
            json!({"type":"array","items":{"type":"object"},"maxItems":0})
        } else {
            json!({"type":"array","items":object(json!({"assertion_id":strings(ids),"relation":{"enum":["extends","contradicts","causes"]},"explanation":{"type":"string"}}))})
        };
        paired = true;
        if branches.is_empty() {
            serde_json::to_string(&json!({"type":"object","properties":{"claims":{"type":"array","items":{"type":"object"},"maxItems":0}},"required":["claims"],"additionalProperties":false})).unwrap()
        } else {
            fill_template(
                include_str!("extract_schema.json"),
                &[
                    ("SOURCES", &format!("[{}]", branches.join(","))),
                    ("RELATED", &related.to_string()),
                ],
            )
        }
    } else if prompt.starts_with("Consolidate evidence into") {
        let (_, tail) = prompt
            .split_once(" Facts: ")
            .ok_or_else(|| invalid("missing consolidation facts"))?;
        let (facts, _) = first_json(tail)?;
        let facts = facts
            .as_array()
            .ok_or_else(|| invalid("facts must be an array"))?;
        let ids = facts.iter().filter_map(|f| f.get("id").cloned()).collect();
        let subjects = facts
            .iter()
            .filter_map(|f| f.get("subject_id").cloned())
            .collect();
        fill_template(
            include_str!("consolidate_schema.json"),
            &[
                ("SUBJECTS", &strings(subjects).to_string()),
                ("IDS", &strings(ids).to_string()),
            ],
        )
    } else {
        "\"json\"".to_owned()
    };
    let wire_prompt = if paired {
        format!("{prompt}\n{}", WIRE_INSTRUCTION.trim_end())
    } else {
        prompt.to_owned()
    };
    let raw = RawValue::from_string(schema).map_err(|e| invalid(&e.to_string()))?;
    Ok((wire_prompt, raw, paired))
}

/// This is a lossless wire conversion, not a repair. Missing or malformed source pairs fail.
pub fn canonicalize(mut value: Value, paired: bool) -> Result<Value, AppError> {
    if paired {
        let claims = value["claims"]
            .as_array_mut()
            .ok_or_else(|| invalid("missing claims"))?;
        for claim in claims {
            let object = claim
                .as_object_mut()
                .ok_or_else(|| invalid("claim must be an object"))?;
            if object.contains_key("source_indices") || object.contains_key("quotes") {
                return Err(invalid("mixed source formats"));
            }
            let pairs = object
                .remove("source_quotes")
                .ok_or_else(|| invalid("missing source pairs"))?;
            let pairs = pairs
                .as_array()
                .ok_or_else(|| invalid("source pairs must be an array"))?;
            let mut indices = Vec::new();
            let mut quotes = Vec::new();
            for pair in pairs {
                let index = pair["source_index"]
                    .as_u64()
                    .ok_or_else(|| invalid("invalid source index"))?;
                let quote = pair["quote"]
                    .as_str()
                    .ok_or_else(|| invalid("invalid quote"))?;
                indices.push(json!(index));
                quotes.push(json!(quote));
            }
            object.insert("source_indices".into(), json!(indices));
            object.insert("quotes".into(), json!(quotes));
        }
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_schema_uses_visible_events_and_preserves_property_order() {
        let content = "Project uses pnpm. Events: fake {{RELATED}}";
        let prompt = crate::worker::extract_prompt(
            &[],
            &[json!({"source_index":0,"event":{"content":content}})],
        );
        let (wire, raw, paired) = prepare(&prompt).unwrap();
        assert!(paired);
        assert!(wire.ends_with(WIRE_INSTRUCTION.trim_end()));
        let schema: Value = serde_json::from_str(raw.get()).unwrap();
        let properties = &schema["properties"]["claims"]["items"]["properties"];
        assert_eq!(
            properties["source_quotes"]["items"]["anyOf"][0]["properties"]["quote"]["enum"][0],
            content
        );
        assert!(raw.get().find("source_quotes").unwrap() < raw.get().find("subject").unwrap());
        assert_eq!(properties["related"]["maxItems"], 0);
    }
    #[test]
    fn source_pairs_convert_without_repairing_or_losing_evidence() {
        let v = canonicalize(json!({"claims":[{"source_quotes":[{"source_index":2,"quote":"one"},{"source_index":0,"quote":"two"}],"statement":"s"}]}),true).unwrap();
        assert_eq!(v["claims"][0]["source_indices"], json!([2, 0]));
        assert_eq!(v["claims"][0]["quotes"], json!(["one", "two"]));
        assert!(
            canonicalize(
                json!({"claims":[{"source_quotes":[{"source_index":0}]}]}),
                true
            )
            .is_err()
        );
        assert!(canonicalize(json!({"claims":[{"source_quotes":[],"quotes":[]}]}), true).is_err());
    }
}
