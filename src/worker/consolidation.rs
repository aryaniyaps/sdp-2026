//! Derive inferences only from supported assertions. The prompt and the model's answer keep
//! the trained names (`FACTS`, `observations`); see `crate::knowledge::kind_to_model`.

use super::*;

/// The consolidation prompt text; `slm-distill/aux_tasks.py` renders the same file.
pub const CONSOLIDATE_TEMPLATE: &str = include_str!("consolidate_prompt.txt");

pub fn consolidate_prompt(assertions: &[Value]) -> String {
    fill_template(
        CONSOLIDATE_TEMPLATE.trim_end_matches('\n'),
        &[(
            "FACTS",
            &serde_json::to_string(assertions).expect("assertions serialize"),
        )],
    )
}

pub(super) async fn consolidate(state: &AppState, job: &Job) -> Result<Value, AppError> {
    let mut assertions:Vec<Value>=sqlx::query_scalar(r#"
SELECT jsonb_build_object('id', a.id, 'subject_id', a.subject_id, 'subject', e.name, 'statement', a.statement, 'kind', a.kind, 'valid_from', a.valid_from)
FROM assertions a
JOIN entities e ON e.id = a.subject_id
WHERE a.namespace = $1
  AND a.status = 'active'
  AND a.kind <> 'inference'
  AND (a.subject_id IN
         (SELECT subject_id
          FROM assertions
          WHERE episode_id = ($2 ->> 'episode_id')::UUID)
       OR a.subject_id IN
         (SELECT subject_id
          FROM assertions
          WHERE id = ($2 ->> 'changed_assertion_id')::UUID))
ORDER BY a.valid_from DESC,
         a.id
LIMIT 100
"#).bind(&job.namespace).bind(&job.payload).fetch_all(&state.store.pool).await?;
    for assertion in &mut assertions {
        let kind = json!(crate::knowledge::kind_to_model(
            assertion["kind"].as_str().unwrap_or_default()
        ));
        assertion["kind"] = kind;
    }
    // A long conversation gives its speakers hundreds of assertions; the newest that fit are kept.
    if let Some(limits) = state.model.context_limits() {
        let (kept, dropped) = fit_newest(&assertions, &limits, consolidate_prompt);
        if dropped > 0 {
            tracing::warn!(
                namespace=%job.namespace, job_id=%job.id, dropped, kept=kept.len(),
                "oldest assertions left out of consolidation to fit the local model context"
            );
        }
        assertions = kept;
    }
    if assertions.len() < 2 {
        return Ok(json!({"inference_ids":[],"reason":"fewer than two active assertions"}));
    }
    let prompt = consolidate_prompt(&assertions);
    let mut input = prompt.clone();
    let mut validated = None;
    let mut last_validation_error = String::new();
    let mut tried: Vec<String> = Vec::new();
    for repair in 0..3 {
        let output = cached_generate(
            &state.store,
            state.model.as_ref(),
            CONSOLIDATE_VERSION,
            &input,
        )
        .await?;
        let validation = serde_json::from_value::<Consolidation>(output.clone())
            .map_err(|e| AppError::Provider(format!("consolidation schema: {e}")))
            .and_then(|parsed| {
                validate_consolidation(&parsed, &assertions)?;
                Ok(parsed)
            });
        match validation {
            Ok(parsed) => {
                validated = Some(parsed);
                break;
            }
            Err(e) => {
                last_validation_error = e.to_string();
                tried.push(input.clone());
                input = format!(
                    "{prompt}\nRepair attempt {repair}. Previous response: {output}\nValidation error: {e}. Return complete corrected JSON. Copy support and subject UUIDs exactly from the supplied facts; use two distinct supports with at least one belonging to the observation subject. Preserve supported observations and omit only unsupported inferences."
                );
            }
        }
    }
    if validated.is_none() {
        for attempt in &tried {
            forget_generated(
                &state.store,
                state.model.as_ref(),
                CONSOLIDATE_VERSION,
                attempt,
            )
            .await?;
        }
    }
    let parsed = validated.ok_or_else(|| {
        AppError::Provider(format!(
            "consolidation failed validation after two repair attempts: {last_validation_error}"
        ))
    })?;
    let mut vectors = Vec::new();
    for o in &parsed.inferences {
        vectors.push(state.embedder.embed(&o.statement).await.ok());
    }
    state
        .store
        .apply_consolidation(job, &parsed, vectors, &state.model.identity())
        .await
}

/// Errors go back to the model in a repair prompt, so they use the names it was trained on.
fn validate_consolidation(parsed: &Consolidation, assertions: &[Value]) -> Result<(), AppError> {
    for inference in &parsed.inferences {
        let supports: std::collections::HashSet<_> = inference.supports.iter().copied().collect();
        if supports.len() < 2
            || !inference.confidence.is_finite()
            || !(0.0..=1.0).contains(&inference.confidence)
            || [&inference.statement, &inference.predicate, &inference.value]
                .iter()
                .any(|s| s.trim().is_empty())
        {
            return Err(AppError::Validation(
                "observation requires two distinct supports, valid confidence and nonempty fields"
                    .into(),
            ));
        }
        if supports.iter().any(|id| {
            !assertions
                .iter()
                .any(|assertion| assertion["id"].as_str() == Some(&id.to_string()))
        }) {
            return Err(AppError::Validation(
                "observation support must come from the provided facts".into(),
            ));
        }
        if !assertions.iter().any(|assertion| {
            assertion["subject_id"].as_str() == Some(&inference.subject_id.to_string())
                && supports
                    .iter()
                    .any(|id| assertion["id"].as_str() == Some(&id.to_string()))
        }) {
            return Err(AppError::Validation(
                "observation subject must belong to at least one supplied support".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod prompt_tests {
    use super::*;

    #[test]
    fn consolidate_prompt_matches_the_fixture_shared_with_the_training_code() {
        let fixture: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/extract_prompt_parity.json"
        ))
        .unwrap();
        for case in fixture["consolidate"].as_array().unwrap() {
            let assertions = case["facts"].as_array().unwrap();
            assert_eq!(
                consolidate_prompt(assertions),
                case["prompt"].as_str().unwrap(),
                "case {}",
                case["name"]
            );
        }
    }
}
