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
    let event_values: Vec<Value> = events
        .iter()
        .enumerate()
        .map(|(index, event)| json!({"source_index":index,"event":event}))
        .collect();
    // Only a model with a known context window gets a shrunk snapshot. Hosted models
    // see every existing fact, so their prompt is unchanged.
    let (existing, dropped_facts) = match state.model.context_limits() {
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
        None => (existing, 0),
    };
    let prompt = extract_prompt(&existing, &event_values);
    let mut input = prompt.clone();
    let mut validated = None;
    let mut last_validation_error = String::new();
    let mut tried: Vec<String> = Vec::new();
    let validation_events: Vec<_> = events.iter().map(|e| (*e).clone()).collect();
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
                    validate_claim(claim, &validation_events)?;
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
    let parsed = validated.ok_or_else(|| {
        AppError::Provider(format!(
            "extraction failed validation after two repair attempts: {last_validation_error}"
        ))
    })?;
    embed_missing_chunks(state, &evidence).await?;
    let mut vectors = Vec::new();
    for claim in &parsed.claims {
        vectors.push(state.embedder.embed(&claim.statement).await.ok());
    }
    let result = state
        .store
        .apply_extraction(job, &parsed, vectors, &state.model.identity())
        .await?;
    let mut outcome = json!({"enhanced":result});
    if dropped_facts > 0 {
        outcome["existing_facts_dropped"] = json!(dropped_facts);
    }
    Ok(outcome)
}
