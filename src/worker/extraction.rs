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
                input = format!(
                    "{prompt}\nRepair attempt {repair}. Previous response: {output}\nValidation error: {e}. Return the complete corrected JSON, preserving all valid claims and correcting attribution/quotes from the original events."
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
