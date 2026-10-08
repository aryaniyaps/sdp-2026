//! Extract facts from evidence and validate every quoted source.

use super::*;

pub(super) async fn extract(state: &AppState, job: &Job) -> Result<Value, AppError> {
    let episode: Uuid = serde_json::from_value(job.payload["episode_id"].clone())
        .map_err(|e| AppError::Validation(e.to_string()))?;
    let evidence = state
        .store
        .episode_evidence(&job.namespace, episode)
        .await?;
    let events: Vec<_> = evidence.iter().map(|(_, e)| e).collect();
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
  AND a.kind <> 'observation'
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
    let ranges = windows(&compact);
    let mut claims = Vec::new();
    let mut dropped_facts = 0;
    for range in &ranges {
        let event_values: Vec<Value> = compact[range.clone()]
            .iter()
            .enumerate()
            .map(|(index, event)| json!({"source_index":index,"event":event}))
            .collect();
        // Only a model with a known context window gets a shrunk snapshot.
        let (window_existing, dropped) = match state.model.context_limits() {
            Some(limits) => {
                let (kept, dropped) = fit_existing_facts(&existing, &event_values, &limits);
                if dropped > 0 {
                    tracing::warn!(
                        namespace=%job.namespace, job_id=%job.id, dropped, kept=kept.len(),
                        num_ctx=limits.num_ctx,
                        "existing facts shrunk to fit the local model context; raise OLLAMA_NUM_CTX to keep more"
                    );
                }
                (kept, dropped)
            }
            None => (existing.clone(), 0),
        };
        dropped_facts += dropped;
        let window_events: Vec<EvidenceEvent> =
            events[range.clone()].iter().map(|e| (*e).clone()).collect();
        let extraction =
            extract_window(state, &window_existing, &event_values, &window_events).await?;
        // Window-local source indices become indices into the whole episode.
        for mut claim in extraction.claims {
            if leaks_secret(&claim.statement) {
                tracing::warn!(
                    namespace=%job.namespace, job_id=%job.id,
                    "dropped an extracted claim that contains a credential"
                );
                continue;
            }
            for index in &mut claim.source_indices {
                *index += range.start;
            }
            claims.push(claim);
        }
    }
    let parsed = Extraction { claims };
    embed_missing_chunks(state, &evidence).await?;
    let mut vectors = Vec::new();
    for claim in &parsed.claims {
        vectors.push(state.embedder.embed(&claim.statement).await.ok());
    }
    let result = state
        .store
        .apply_extraction(job, &parsed, vectors, &state.model.identity())
        .await?;
    let mut outcome = json!({"enhanced":result,"windows":ranges.len()});
    if dropped_facts > 0 {
        outcome["existing_facts_dropped"] = json!(dropped_facts);
    }
    Ok(outcome)
}

/// Ask the model about one window and return claims whose quotes are verified against the window's
/// full events. A reply that fails validation is sent back with the error, twice at most.
async fn extract_window(
    state: &AppState,
    existing: &[Value],
    event_values: &[Value],
    validation_events: &[EvidenceEvent],
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
                        !existing.iter().any(|fact| {
                            fact["id"].as_str() == Some(&relation.assertion_id.to_string())
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

/// Keep all original source evidence when an invalid answer is too large to echo back.
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
    fn oversized_invalid_answer_keeps_all_evidence_and_fits_on_wire() {
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
