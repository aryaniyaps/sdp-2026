//! Extract assertions from source events and validate every quoted source.

use super::*;

pub(super) async fn extract(state: &AppState, job: &Job) -> Result<Value, AppError> {
    let episode: Uuid = serde_json::from_value(job.payload["episode_id"].clone())
        .map_err(|e| AppError::Validation(e.to_string()))?;
    let sources = state.store.episode_events(&job.namespace, episode).await?;
    // Keep the complete ledger and its indices; recalled assistant answers are
    // derived from memory, not a new independent source for assertion extraction.
    let indexed_events: Vec<_> = sources
        .iter()
        .enumerate()
        .filter(|(_, (_, event))| extraction_source(event))
        .map(|(index, (_, event))| (index, event))
        .collect();
    let events: Vec<_> = indexed_events.iter().map(|(_, event)| *event).collect();
    let existing: Vec<Value> = if let Some(snapshot) = job.payload.get("existing_snapshot") {
        serde_json::from_value(snapshot.clone()).map_err(|e| AppError::Validation(e.to_string()))?
    } else {
        let snapshot:Vec<Value>=sqlx::query_scalar(r#"
SELECT jsonb_build_object('id', a.id, 'subject', e.name, 'subject_id', e.id, 'predicate', a.predicate, 'value', a.value, 'cardinality', a.cardinality, 'valid_from', a.valid_from)
FROM assertions a
JOIN entities e ON e.id = a.subject_id
WHERE a.namespace = $1
  AND a.status IN ('active',
           'contested')
  AND a.kind <> 'inference'
ORDER BY a.valid_from DESC,
         a.id
LIMIT 80
"#).bind(&job.namespace).fetch_all(&state.store.pool).await?;
        let updated = sqlx::query("UPDATE memory_jobs SET payload=payload || jsonb_build_object('existing_snapshot',$3::jsonb) WHERE id=$1 AND lease_token=$2 AND status='running'").bind(job.id).bind(job.lease_token).bind(json!(snapshot)).execute(&state.store.pool).await?;
        if updated.rows_affected() != 1 {
            return Err(AppError::Conflict("extraction lease lost".into()));
        }
        snapshot
    };
    // The model reads shortened events in windows of a few thousand tokens. Claims are still
    // checked against the full events, so a quote from the part that was shown is accepted.
    let compact: Vec<Value> = events.iter().map(|event| compact_event(event)).collect();
    // A window whose claims do not fit the model's output cap is split in half and each half is
    // extracted on its own. Only a single event whose claims still do not fit fails the job.
    let mut pending: Vec<std::ops::Range<usize>> = windows(&compact).into_iter().rev().collect();
    let (mut extracted_windows, mut split_windows) = (0, 0);
    let mut claims = Vec::new();
    let mut dropped_assertions = 0;
    while let Some(range) = pending.pop() {
        let event_values: Vec<Value> = compact[range.clone()]
            .iter()
            .enumerate()
            .map(|(index, event)| json!({"source_index":index,"event":event}))
            .collect();
        // Only a model with a known context window gets a shrunk snapshot.
        let (window_existing, dropped) = match state.model.context_limits() {
            Some(limits) => {
                let (kept, dropped) = fit_existing_assertions(&existing, &event_values, &limits);
                if dropped > 0 {
                    tracing::warn!(
                        namespace=%job.namespace, job_id=%job.id, dropped, kept=kept.len(),
                        num_ctx=limits.num_ctx,
                        "existing assertions shrunk to fit the local model context; raise OLLAMA_NUM_CTX to keep more"
                    );
                }
                (kept, dropped)
            }
            None => (existing.clone(), 0),
        };
        let window_events: Vec<SourceEvent> =
            events[range.clone()].iter().map(|e| (*e).clone()).collect();
        let reviewed =
            match extract_window(state, &window_existing, &event_values, &window_events).await {
                Ok(extraction) => super::review::review(state, extraction, &window_events).await,
                Err(error) => Err(error),
            };
        let extraction = match reviewed {
            Err(AppError::OutputLimit(reason)) if range.len() > 1 => {
                tracing::warn!(
                    namespace=%job.namespace, job_id=%job.id, events=range.len(), %reason,
                    "window answer hit the output cap; extracting its halves separately"
                );
                let middle = range.start + range.len() / 2;
                pending.push(middle..range.end);
                pending.push(range.start..middle);
                split_windows += 1;
                continue;
            }
            other => other?,
        };
        extracted_windows += 1;
        dropped_assertions += dropped;
        // Window-local source indices become indices into the whole episode.
        for mut claim in extraction.claims {
            if leaks_secret(&claim.statement) {
                tracing::warn!(
                    namespace=%job.namespace, job_id=%job.id,
                    "dropped an extracted claim that contains a credential"
                );
                continue;
            }
            resolve_relative_day(&mut claim, &window_events);
            for index in &mut claim.source_indices {
                *index = indexed_events[*index + range.start].0;
            }
            claims.push(claim);
        }
    }
    let parsed = Extraction { claims };
    embed_missing_chunks(state, &sources).await?;
    let mut vectors = Vec::new();
    for claim in &parsed.claims {
        vectors.push(state.embedder.embed(&claim.statement).await.ok());
    }
    let result = state
        .store
        .apply_extraction(job, &parsed, vectors, &state.model.identity())
        .await?;
    let mut outcome = json!({"enhanced":result,"windows":extracted_windows});
    if split_windows > 0 {
        outcome["split_windows"] = json!(split_windows);
    }
    if dropped_assertions > 0 {
        outcome["existing_assertions_dropped"] = json!(dropped_assertions);
    }
    Ok(outcome)
}

/// The student often copies the time a message was sent into `event_at` when the quote says
/// "yesterday", sometimes miscounts "in three days", and sometimes leaves `event_at` empty, which the
/// store fills with the time the message was sent. When the claim's own quotes name exactly one
/// relative day, that day is the event date: in `event_at`, in `valid_from` when it was the same
/// instant, and where the statement spells out the model's date. `source_indices` must still index
/// `events`.
pub(crate) fn resolve_relative_day(claim: &mut ClaimInput, events: &[SourceEvent]) {
    let Some(said) = claim
        .source_indices
        .first()
        .and_then(|&i| events.get(i))
        .map(|e| e.occurred_at)
    else {
        return;
    };
    let event_at = match claim.event_at {
        Some(event_at) => event_at,
        None if claim.cardinality == Cardinality::Event => claim.valid_from.unwrap_or(said),
        None => return,
    };
    let Some(day) = crate::temporal::mentioned_day(&claim.quotes.join(" "), said) else {
        return;
    };
    let given = event_at.date_naive();
    let shift = day - given;
    if shift.is_zero() {
        return;
    }
    claim.event_at = Some(event_at + shift);
    if claim.valid_from == Some(event_at) {
        claim.valid_from = claim.event_at;
    }
    for format in ["%Y-%m-%d", "%-d %B %Y", "%B %-d, %Y"] {
        let (old, new) = (
            given.format(format).to_string(),
            day.format(format).to_string(),
        );
        claim.statement = claim.statement.replace(&old, &new);
    }
}

fn extraction_source(event: &SourceEvent) -> bool {
    !(event.role == "assistant" && event.metadata["memory_derived"] == true)
}

/// Ask the model about one window and return claims whose quotes are verified against the window's
/// full events. A reply that fails validation is sent back with the error, twice at most.
async fn extract_window(
    state: &AppState,
    existing: &[Value],
    event_values: &[Value],
    validation_events: &[SourceEvent],
) -> Result<Extraction, AppError> {
    let prompt = extract_prompt(existing, event_values);
    let mut input = prompt.clone();
    let mut validated = None;
    let mut last_validation_error = String::new();
    let mut tried: Vec<String> = Vec::new();
    for repair in 0..3 {
        let output = match cached_generate(
            &state.store,
            state.model.as_ref(),
            EXTRACT_VERSION,
            &input,
        )
        .await
        {
            Ok(output) => output,
            Err(error) => {
                // A repair prompt that cannot be sent (for example it no longer fits the
                // local context) ends the chain: drop the invalid replies cached so far, or
                // a retry would replay them and fail the same way.
                for attempt in &tried {
                    forget_generated(&state.store, state.model.as_ref(), EXTRACT_VERSION, attempt)
                        .await?;
                }
                return Err(error);
            }
        };
        let validation = serde_json::from_value::<Extraction>(output.clone())
            .map_err(|e| AppError::Provider(format!("extraction schema: {e}")))
            .and_then(|extraction| {
                for claim in &extraction.claims {
                    validate_claim(claim, validation_events)?;
                    if claim.related.iter().any(|relation| {
                        !existing.iter().any(|assertion| {
                            assertion["id"].as_str() == Some(&relation.assertion_id.to_string())
                        })
                    }) {
                        return Err(AppError::Validation(
                            "related assertion must come from the provided existing facts".into(),
                        ));
                    }
                }
                Ok(extraction)
            });
        match validation {
            Ok(extraction) => {
                validated = Some(extraction);
                break;
            }
            Err(e) => {
                last_validation_error = e.to_string();
                tried.push(input.clone());
                input = repair_prompt(
                    &prompt,
                    &output,
                    &e.to_string(),
                    repair,
                    state.model.context_limits(),
                );
            }
        }
    }
    if validated.is_none() {
        // Every reply in the chain was invalid. Drop them so a retry asks the model again;
        // a chain that ended in a valid repair stays cached and replays identically.
        for attempt in &tried {
            forget_generated(&state.store, state.model.as_ref(), EXTRACT_VERSION, attempt).await?;
        }
    }
    validated.ok_or_else(|| {
        AppError::Provider(format!(
            "extraction failed validation after two repair attempts: {last_validation_error}"
        ))
    })
}

/// Keep all original source text when an invalid answer is too large to echo back.
/// A regeneration with a bounded diagnostic is safer than truncating source events or
/// repeatedly sending a repair that the model's preflight will always reject.
fn repair_prompt(
    prompt: &str,
    output: &Value,
    error: &str,
    repair: usize,
    limits: Option<crate::model::OllamaLimits>,
) -> String {
    let full = format!(
        "{prompt}\nRepair attempt {repair}. Previous response: {output}\nValidation error: {error}. Return the complete corrected JSON, preserving all valid claims and correcting attribution/quotes from the original events."
    );
    let Some(limits) = limits else { return full };
    let fits =
        |value: &str| crate::model::extraction_prompt_tokens(value) <= limits.prompt_budget();
    if fits(&full) {
        return full;
    }
    let mut diagnostic: Vec<char> = error.chars().take(512).collect();
    loop {
        let bounded = format!(
            "{prompt}\nRepair attempt {repair}. Validation error: {}. Generate the complete corrected JSON from the original evidence; the invalid previous answer was omitted to fit the context.",
            diagnostic.iter().collect::<String>()
        );
        if fits(&bounded) {
            return bounded;
        }
        if diagnostic.is_empty() {
            // The original request still goes through the model's exact preflight. Never
            // bypass that guard, even if the source prompt itself cannot fit.
            return prompt.to_owned();
        }
        diagnostic.truncate(diagnostic.len() / 2);
    }
}

#[cfg(test)]
mod repair_tests {
    use super::*;

    #[test]
    fn a_relative_day_in_the_quote_moves_a_copied_message_date() {
        use chrono::TimeZone;
        let said = chrono::Utc.with_ymd_and_hms(2023, 5, 8, 13, 56, 0).unwrap();
        let events = vec![SourceEvent {
            role: "user".into(),
            content: "Caroline: I went to a LGBTQ support group yesterday and it was so powerful."
                .into(),
            occurred_at: said,
            metadata: json!({}),
        }];
        let claim = |quote: &str, event_at| ClaimInput {
            subject: EntityInput {
                name: "Caroline".into(),
                entity_type: "person".into(),
                aliases: vec![],
            },
            predicate: "attended".into(),
            value: "LGBTQ support group".into(),
            statement: "Caroline attended an LGBTQ support group on 2023-05-08.".into(),
            cardinality: Cardinality::Event,
            kind: "event".into(),
            confidence: 0.9,
            valid_from: event_at,
            event_at,
            source_indices: vec![0],
            quotes: vec![quote.into()],
            entities: vec![],
            correction: false,
            explanation: String::new(),
            related: vec![],
        };
        let mut copied = claim("I went to a LGBTQ support group yesterday", Some(said));
        resolve_relative_day(&mut copied, &events);
        assert_eq!(
            copied.event_at.unwrap().date_naive().to_string(),
            "2023-05-07"
        );
        assert_eq!(copied.valid_from, copied.event_at);
        assert_eq!(
            copied.statement,
            "Caroline attended an LGBTQ support group on 2023-05-07."
        );
        // A miscounted date is corrected too: "in three days" from 8 May is 11 May, not 12 May.
        let mut miscounted = claim(
            "I will present at the review in three days",
            Some(said + chrono::Duration::days(4)),
        );
        miscounted.statement = "Caroline will present on 2023-05-12.".into();
        resolve_relative_day(&mut miscounted, &events);
        assert_eq!(
            miscounted.event_at.unwrap().date_naive().to_string(),
            "2023-05-11"
        );
        assert_eq!(miscounted.statement, "Caroline will present on 2023-05-11.");
        // An empty event date would become the message time in the store, so it is resolved too.
        let mut empty = claim("I went to a LGBTQ support group yesterday", None);
        empty.statement = "Caroline attended an LGBTQ support group yesterday.".into();
        resolve_relative_day(&mut empty, &events);
        assert_eq!(
            empty.event_at.unwrap().date_naive().to_string(),
            "2023-05-07"
        );
        assert_eq!(empty.valid_from, None);
        // A date the model already resolved, a quote without a relative day, or a state without a
        // date stays as it is.
        let resolved = said - chrono::Duration::days(1);
        for (quote, event_at, cardinality) in [
            (
                "I went to a LGBTQ support group yesterday",
                Some(resolved),
                Cardinality::Event,
            ),
            (
                "I went to a LGBTQ support group",
                Some(said),
                Cardinality::Event,
            ),
            ("I went to a LGBTQ support group", None, Cardinality::Event),
            (
                "I went to a LGBTQ support group yesterday",
                None,
                Cardinality::Multiple,
            ),
        ] {
            let mut unchanged = claim(quote, event_at);
            unchanged.cardinality = cardinality;
            resolve_relative_day(&mut unchanged, &events);
            assert_eq!(unchanged.event_at, event_at, "{quote}");
            assert!(unchanged.statement.ends_with("2023-05-08."), "{quote}");
        }
    }

    #[test]
    fn recalled_answers_are_not_independent_sources_but_new_messages_are() {
        let event = |role: &str, metadata: Value| SourceEvent {
            role: role.into(),
            content: "Mira owns Orion".into(),
            occurred_at: chrono::Utc::now(),
            metadata,
        };
        assert!(!extraction_source(&event(
            "assistant",
            json!({"memory_derived":true})
        )));
        assert!(extraction_source(&event("assistant", json!({}))));
        assert!(extraction_source(&event(
            "user",
            json!({"memory_derived":true})
        )));
        assert!(extraction_source(&event(
            "tool",
            json!({"memory_derived":true})
        )));
    }

    #[test]
    fn oversized_invalid_answer_keeps_all_source_text_and_fits_on_wire() {
        let prompt = extract_prompt(
            &[],
            &[
                json!({"source_index":0,"event":{"role":"user","content":"Maya lives in Chennai.","occurred_at":"2026-10-08T10:00:00Z","metadata":{}}}),
            ],
        );
        let limits = crate::model::OllamaLimits::new(8192, 3072).unwrap();
        let repaired = repair_prompt(
            &prompt,
            &json!({"claims":[{"statement":"x".repeat(40_000)}]}),
            &"invalid quote ".repeat(2000),
            1,
            Some(limits),
        );
        assert!(repaired.starts_with(&prompt));
        assert!(repaired.contains("previous answer was omitted"));
        assert!(crate::model::extraction_prompt_tokens(&repaired) <= limits.prompt_budget());
        assert!(!repaired.contains(&"x".repeat(100)));
    }

    #[test]
    fn small_invalid_answer_remains_available_for_repair() {
        let prompt = extract_prompt(&[], &[]);
        let output = json!({"claims":[]});
        let repaired = repair_prompt(
            &prompt,
            &output,
            "missing evidence",
            0,
            Some(crate::model::OllamaLimits::new(16384, 4096).unwrap()),
        );
        assert!(repaired.starts_with(&prompt));
        assert!(repaired.contains(&format!("Previous response: {output}")));
    }
}
