//! Extraction prompt and the existing-fact budget for local models.

#[cfg(test)]
use crate::model::estimate_tokens;
use crate::model::{OllamaLimits, extraction_prompt_tokens};
use serde_json::Value;

/// Tokens kept free for a repair prompt, which appends the previous answer and the validation
/// error to the original prompt.
const REPAIR_RESERVE_TOKENS: usize = 1536;

/// Most prompt tokens an extraction call is built to use, whatever the context window allows.
/// The student is trained on prompts of this size; a longer snapshot of existing facts only
/// slows every call down.
const MAX_PROMPT_TOKENS: usize = 6000;

/// Fill `{{NAME}}` placeholders in one pass. A value is never scanned for placeholders, so a fact
/// or a quote that happens to contain `{{EVENTS}}` cannot change the prompt. Panics on a
/// placeholder without a value: the templates are files in this repository.
pub fn fill_template(template: &str, values: &[(&str, &str)]) -> String {
    let mut out =
        String::with_capacity(template.len() + values.iter().map(|v| v.1.len()).sum::<usize>());
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after
            .find("}}")
            .expect("unterminated placeholder in a prompt template");
        let name = &after[..end];
        let value = values
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .unwrap_or_else(|| panic!("no value for placeholder {name} in a prompt template"))
            .1;
        out.push_str(value);
        rest = &after[end + 2..];
    }
    out.push_str(rest);
    out
}

/// The extract prompt text. `{{EXISTING}}` and `{{EVENTS}}` mark where the two JSON lists go.
/// The same file is read by `slm-distill/prompt.py`, so the student is trained on exactly the
/// text it is served.
pub const EXTRACT_TEMPLATE: &str = include_str!("extract_prompt.txt");

/// The extract prompt. `events` are the `{"source_index","event"}` objects of one window.
pub fn extract_prompt(existing: &[Value], events: &[Value]) -> String {
    fill_template(
        EXTRACT_TEMPLATE.trim_end_matches('\n'),
        &[
            (
                "EXISTING",
                &serde_json::to_string(existing).expect("existing facts serialize"),
            ),
            (
                "EVENTS",
                &serde_json::to_string(events).expect("events serialize"),
            ),
        ],
    )
}

/// Lowercase words of a text: maximal runs of alphanumeric characters.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Words of the event content in the batch. The JSON keys, roles and timestamps around the
/// content are not evidence, so they never count as a mention.
fn batch_content_words(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| event["event"]["content"].as_str())
        .flat_map(words)
        .collect()
}

/// How strongly a fact is referred to by the batch: its subject counts 3, its value 2, and a
/// text of fewer than three characters (`go`, `5`) counts 1, because such a text is a word in
/// many unrelated sentences. A text counts only when all its words appear next to each other.
fn mention_strength(fact: &Value, batch_words: &[String]) -> u8 {
    let score = |key: &str, weight: u8| -> u8 {
        let text = words(fact[key].as_str().unwrap_or(""));
        if text.is_empty() || !batch_words.windows(text.len()).any(|window| window == text) {
            return 0;
        }
        if text.iter().map(|word| word.chars().count()).sum::<usize>() < 3 {
            1
        } else {
            weight
        }
    };
    score("subject", 3) + score("value", 2)
}

/// Shrink the existing-facts snapshot until the extract prompt fits a local model's context,
/// leaving room for the answer and a repair round. Returns the kept facts in their original
/// order and how many were dropped. Facts whose subject or value appears as whole words in the
/// content of this batch's events are kept first, strongest match first (subject and value, then
/// subject, then value; a text shorter than three characters counts for less), so a correction
/// of such a fact still finds the fact it replaces. The rest are kept newest first, which is the
/// snapshot order. A correction that refers to its target only by pronoun cannot be matched by
/// text and is protected only by recency. Pure and deterministic, so a retry builds the
/// identical prompt and hits the model cache.
pub fn fit_existing_facts(
    existing: &[Value],
    events: &[Value],
    limits: &OllamaLimits,
) -> (Vec<Value>, usize) {
    let budget = limits.prompt_budget();
    let target = budget
        .saturating_sub(REPAIR_RESERVE_TOKENS.min(budget / 4))
        .min(MAX_PROMPT_TOKENS);
    let fits = |facts: &[Value]| extraction_prompt_tokens(&extract_prompt(facts, events)) <= target;
    if fits(existing) {
        return (existing.to_vec(), 0);
    }
    let batch_words = batch_content_words(events);
    let strengths: Vec<u8> = existing
        .iter()
        .map(|fact| mention_strength(fact, &batch_words))
        .collect();
    let mut order: Vec<usize> = (0..existing.len()).collect();
    // Stable sort: equal strengths keep the snapshot order, which is newest first.
    order.sort_by_key(|&index| std::cmp::Reverse(strengths[index]));
    let select = |count: usize| -> Vec<Value> {
        let mut chosen = order[..count].to_vec();
        chosen.sort_unstable();
        chosen
            .into_iter()
            .map(|index| existing[index].clone())
            .collect()
    };
    // More facts never shorten the prompt, so the largest fitting count is found by bisection.
    let (mut low, mut high) = (0, existing.len());
    while low < high {
        let middle = (low + high).div_ceil(2);
        if fits(&select(middle)) {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    (select(low), existing.len() - low)
}

/// Give retained source chunks a vector as soon as possible so semantic recall
/// works before extraction finishes. A provider outage is logged and leaves the
/// chunk unembedded; the namespace status reports the gap and the extract job
/// tries again.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts(count: usize) -> Vec<Value> {
        (0..count)
            .map(|n| {
                json!({"id":format!("00000000-0000-4000-8000-{n:012}"),"subject":format!("Subject {n}"),"subject_id":format!("10000000-0000-4000-8000-{n:012}"),"predicate":"likes","value":format!("thing number {n} with some padding words"),"cardinality":"multiple","valid_from":"2026-01-02T00:00:00Z"})
            })
            .collect()
    }
    fn batch(text: &str) -> Vec<Value> {
        vec![
            json!({"source_index":0,"event":{"role":"user","content":text,"occurred_at":"2026-02-03T04:05:06Z","metadata":null}}),
        ]
    }

    #[test]
    fn prompt_is_the_template_with_both_lists_filled_in() {
        let existing = facts(2);
        let events = batch("I moved to Lisbon. \"quoted\" and unicode \u{e9}");
        let prompt = extract_prompt(&existing, &events);
        assert!(!prompt.contains("{{"), "a placeholder was left unfilled");
        assert!(prompt.ends_with(&format!(
            "Existing facts: {}\nEvents: {}",
            serde_json::to_string(&existing).unwrap(),
            serde_json::to_string(&events).unwrap()
        )));
        assert!(prompt.starts_with("You are the extraction worker"));
        assert!(!prompt.ends_with('\n'));
    }
    #[test]
    fn prompt_matches_the_fixture_shared_with_the_training_code() {
        // slm-distill/test_prompt_parity.py renders the same input and compares with this file,
        // so the served prompt and the training prompt cannot drift apart.
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/extract_prompt_parity.json"
        ))
        .unwrap();
        for case in fixture["cases"].as_array().unwrap() {
            let existing = case["existing"].as_array().unwrap();
            let events: Vec<Value> = case["events"]
                .as_array()
                .unwrap()
                .iter()
                .enumerate()
                .map(|(index, event)| json!({"source_index":index,"event":event}))
                .collect();
            assert_eq!(
                extract_prompt(existing, &events),
                case["prompt"].as_str().unwrap(),
                "case {}",
                case["name"]
            );
        }
    }
    #[test]
    fn a_value_that_looks_like_a_placeholder_changes_nothing() {
        let existing = vec![json!({"value":"{{EVENTS}} and {{NOPE}}"})];
        let events = batch("hello");
        let prompt = extract_prompt(&existing, &events);
        assert_eq!(prompt.matches("{{EVENTS}} and {{NOPE}}").count(), 1);
        assert!(prompt.ends_with(&format!(
            "Events: {}",
            serde_json::to_string(&events).unwrap()
        )));
        assert_eq!(
            fill_template("a {{X}} b {{Y}}", &[("X", "{{Y}}"), ("Y", "2")]),
            "a {{Y}} b 2"
        );
    }
    #[test]
    #[should_panic(expected = "no value for placeholder Z")]
    fn an_unknown_placeholder_is_a_loud_error() {
        fill_template("{{Z}}", &[("X", "1")]);
    }
    #[test]
    fn snapshot_that_fits_is_left_alone() {
        let limits = OllamaLimits::new(16384, 4096).unwrap();
        let existing = facts(10);
        let (kept, dropped) = fit_existing_facts(&existing, &batch("hello"), &limits);
        assert_eq!(dropped, 0);
        assert_eq!(kept, existing);
    }
    #[test]
    fn oversized_snapshot_is_shrunk_to_the_budget_and_counts_drops() {
        // The expanded general-purpose prompt plus wire instructions consumes the
        // repair-adjusted 4096-token context on its own. Use 5120 to test ranking
        // when there is actually room for a nonempty snapshot.
        let limits = OllamaLimits::new(5120, 1024).unwrap();
        let existing = facts(80);
        let events = batch("hello");
        assert!(estimate_tokens(&extract_prompt(&existing, &events)) > limits.prompt_budget());
        let (kept, dropped) = fit_existing_facts(&existing, &events, &limits);
        assert!(dropped > 0 && !kept.is_empty());
        assert_eq!(kept.len() + dropped, 80);
        let prompt = extract_prompt(&kept, &events);
        assert!(
            extraction_prompt_tokens(&prompt) + limits.num_predict <= limits.num_ctx,
            "wire prompt must leave room for the answer"
        );
        // Nothing but the oldest facts goes: the snapshot is newest first, so a kept prefix remains
        // in the original order.
        assert_eq!(kept, existing[..kept.len()].to_vec());
        // One more fact would no longer fit the repair-adjusted target.
        let mut one_more = kept.clone();
        one_more.push(existing[kept.len()].clone());
        let budget = limits.prompt_budget();
        let target = budget - REPAIR_RESERVE_TOKENS.min(budget / 4);
        assert!(extraction_prompt_tokens(&extract_prompt(&one_more, &events)) > target);
    }
    #[test]
    fn small_context_drops_snapshot_but_keeps_source_prompt_within_wire_budget() {
        let limits = OllamaLimits::new(4096, 1024).unwrap();
        let existing = facts(80);
        let events = batch("Maya lives in Chennai.");
        let (kept, dropped) = fit_existing_facts(&existing, &events, &limits);
        assert!(kept.is_empty());
        assert_eq!(dropped, existing.len());
        let prompt = extract_prompt(&kept, &events);
        assert!(prompt.contains("Maya lives in Chennai."));
        assert!(extraction_prompt_tokens(&prompt) <= limits.prompt_budget());
    }
    #[test]
    fn facts_mentioned_in_the_batch_survive_so_corrections_still_resolve() {
        let limits = OllamaLimits::new(5120, 1024).unwrap();
        let existing = facts(80);
        let events = batch("Actually Subject 77 now prefers tea.");
        let (kept, dropped) = fit_existing_facts(&existing, &events, &limits);
        assert!(dropped > 0);
        assert!(
            kept.contains(&existing[77]),
            "the fact the batch corrects must stay"
        );
        let positions: Vec<usize> = kept
            .iter()
            .map(|fact| existing.iter().position(|e| e == fact).unwrap())
            .collect();
        assert!(
            positions.windows(2).all(|w| w[0] < w[1]),
            "original order kept"
        );
    }
    fn corrected_fact_scenario(text: &str) -> (Vec<Value>, Vec<Value>, OllamaLimits) {
        let limits = OllamaLimits::new(5120, 1024).unwrap();
        let mut existing: Vec<Value> = (0..79)
            .map(|n| {
                let value = if n < 60 {
                    "go".to_string()
                } else {
                    format!("thing number {n} with some padding words")
                };
                json!({"id":format!("00000000-0000-4000-8000-{n:012}"),"subject":format!("Subject {n}"),"subject_id":format!("10000000-0000-4000-8000-{n:012}"),"predicate":"likes","value":value,"cardinality":"multiple","valid_from":"2026-01-02T00:00:00Z"})
            })
            .collect();
        // The oldest fact is the one the batch corrects.
        existing.push(json!({"id":"00000000-0000-4000-8000-0000000000ff","subject":"Haz","subject_id":"10000000-0000-4000-8000-0000000000ff","predicate":"lives_in","value":"Porto","cardinality":"single","valid_from":"2025-01-02T00:00:00Z"}));
        (existing, batch(text), limits)
    }
    #[test]
    fn short_values_inside_ordinary_words_do_not_crowd_out_the_corrected_fact() {
        let (existing, events, limits) =
            corrected_fact_scenario("I am going to move from Porto to Lisbon, Haz said, good news");
        let (kept, dropped) = fit_existing_facts(&existing, &events, &limits);
        assert!(dropped > 0);
        assert!(
            kept.contains(&existing[79]),
            "the Porto fact the batch corrects must stay"
        );
    }
    #[test]
    fn short_values_that_are_whole_words_rank_below_a_subject_and_value_hit() {
        let (existing, events, limits) =
            corrected_fact_scenario("Let us go. I move from Porto to Lisbon, Haz said, go go");
        let (kept, dropped) = fit_existing_facts(&existing, &events, &limits);
        assert!(dropped > 0);
        assert!(
            kept.contains(&existing[79]),
            "a subject and value hit outranks 60 facts that only share the word go"
        );
    }
    #[test]
    fn json_keys_and_roles_around_the_content_are_not_mentions() {
        let facts = [
            json!({"subject":"user","value":"event"}),
            json!({"subject":"role","value":"content"}),
            json!({"subject":"source_index","value":"2026"}),
        ];
        let words = batch_content_words(&batch("hello there"));
        for fact in &facts {
            assert_eq!(mention_strength(fact, &words), 0, "{fact}");
        }
    }
    #[test]
    fn mention_strength_ranks_subject_and_value_over_subject_over_value() {
        let words = batch_content_words(&batch("Haz left Porto. Ana likes Porto, and 5 apples."));
        let both = json!({"subject":"Haz","value":"Porto"});
        let subject = json!({"subject":"Haz","value":"Madrid"});
        let value = json!({"subject":"Bob","value":"porto"});
        let short = json!({"subject":"Bob","value":"5"});
        let partial = json!({"subject":"Bob","value":"Port"});
        let (b, s, v, sh, p) = (
            mention_strength(&both, &words),
            mention_strength(&subject, &words),
            mention_strength(&value, &words),
            mention_strength(&short, &words),
            mention_strength(&partial, &words),
        );
        assert!(b > s && s > v && v > sh && sh > p, "{b} {s} {v} {sh} {p}");
        assert_eq!(p, 0, "a prefix of a word is not a mention");
    }
    #[test]
    fn multi_word_texts_must_match_next_to_each_other() {
        let words = batch_content_words(&batch("Subject went home to number 7"));
        assert_eq!(mention_strength(&json!({"subject":"Subject 7"}), &words), 0);
        let words = batch_content_words(&batch("Subject 7 went home"));
        assert_eq!(mention_strength(&json!({"subject":"Subject 7"}), &words), 3);
    }
    #[test]
    fn shrink_is_deterministic() {
        let limits = OllamaLimits::new(5120, 1024).unwrap();
        let existing = facts(80);
        let events = batch("Subject 5 and Subject 70");
        assert_eq!(
            fit_existing_facts(&existing, &events, &limits),
            fit_existing_facts(&existing, &events, &limits)
        );
    }
    #[test]
    fn events_that_alone_exceed_the_budget_leave_no_facts() {
        let limits = OllamaLimits::new(5120, 1024).unwrap();
        let events = batch(&"word ".repeat(5000));
        let (kept, dropped) = fit_existing_facts(&facts(5), &events, &limits);
        assert!(kept.is_empty());
        assert_eq!(dropped, 5);
        // The model preflight, not this function, then refuses the prompt loudly.
        assert!(limits.preflight(&extract_prompt(&kept, &events)).is_err());
    }
}
