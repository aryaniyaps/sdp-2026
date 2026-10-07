//! Answers grounded in the evidence returned by recall.

use super::{RecallRequest, recall_engine};
use crate::{AppError, AppState, model::cached_generate, observability::TraceBuilder};
use serde_json::{Value, json};
use std::{collections::HashSet, time::Instant};
use uuid::Uuid;

pub async fn reflect_engine(s: &AppState, r: RecallRequest) -> Result<Value, AppError> {
    let mut trace = TraceBuilder::new("reflect_v2", &r.namespace, None, json!(r));
    let started = Instant::now();
    let mut result = perform_reflect(s, r).await;
    trace.step(
        "cited_reflection",
        if result.is_ok() { "ok" } else { "error" },
        started.elapsed(),
        match &result {
            Ok(value) => value.clone(),
            Err(error) => json!({"error":error.to_string()}),
        },
    );
    let trace_id = trace.id();
    let degraded = result
        .as_ref()
        .ok()
        .and_then(|v| v["recall"]["degraded_reasons"].as_array())
        .is_some_and(|reasons| !reasons.is_empty());
    s.store
        .record_trace(&trace.finish(
            if result.is_ok() {
                if degraded { "degraded" } else { "succeeded" }
            } else {
                "failed"
            },
            degraded,
            result.as_ref().ok().cloned(),
            result.as_ref().err().map(ToString::to_string),
        ))
        .await?;
    if let Ok(value) = &mut result {
        value["trace_id"] = json!(trace_id);
    }
    result
}
async fn perform_reflect(s: &AppState, r: RecallRequest) -> Result<Value, AppError> {
    let query = r.query.clone();
    let recall = recall_engine(s, r).await?;
    if recall.results.is_empty() {
        return Ok(
            json!({"answer":"Insufficient evidence.","citations":[],"insufficient_evidence":true,"recall":recall}),
        );
    }
    let prompt = format!(
        "Answer only from the following cited evidence. Contested assertions are not established facts. Return JSON {{\"answer\":\"...\",\"citations\":[\"retrieved UUID\"],\"insufficient_evidence\":false}}. If evidence is insufficient set insufficient_evidence=true. Never cite an ID not in the evidence. Question: {}\nEvidence:\n{}",
        serde_json::to_string(&query).unwrap(),
        recall.context
    );
    let output = cached_generate(&s.store, s.model.as_ref(), "reflect-v2.1", &prompt).await?;
    let citations: Vec<Uuid> = serde_json::from_value(output["citations"].clone())
        .map_err(|e| AppError::Provider(e.to_string()))?;
    let allowed: HashSet<_> = recall.results.iter().map(|h| h.id).collect();
    if citations.iter().any(|id| !allowed.contains(id))
        || output["answer"].as_str().is_none()
        || output["insufficient_evidence"].as_bool().is_none()
        || (output["insufficient_evidence"] == false && citations.is_empty())
    {
        return Err(AppError::Provider(
            "reflection has invalid or missing citations".into(),
        ));
    }
    Ok(
        json!({"answer":output["answer"],"citations":citations,"insufficient_evidence":output["insufficient_evidence"],"recall":recall}),
    )
}
