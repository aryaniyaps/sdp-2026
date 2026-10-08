//! Background job loop and extraction, consolidation, and projection processing.

use crate::{
    AppError, AppState,
    knowledge::*,
    model::{cached_generate, forget_generated},
    observability::TraceBuilder,
};
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;

mod consolidation;
mod extraction;
mod projection;
mod prompts;
mod window;
pub use consolidation::{CONSOLIDATE_TEMPLATE, consolidate_prompt};
pub use prompts::{EXTRACT_TEMPLATE, extract_prompt, fill_template, fit_existing_facts};
pub use window::{WINDOW_EVENT_TOKENS, compact_event, shorten_content, windows};

const EXTRACT_VERSION: &str = "extract-v3.0";
const CONSOLIDATE_VERSION: &str = "consolidate-v2.3";

pub async fn embed_missing_chunks(
    state: &AppState,
    chunks: &[(Uuid, EvidenceEvent)],
) -> Result<(), AppError> {
    for (chunk, event) in chunks {
        let missing: bool = sqlx::query_scalar("SELECT embedding IS NULL FROM chunks WHERE id=$1")
            .bind(chunk)
            .fetch_one(&state.store.pool)
            .await?;
        if !missing {
            continue;
        }
        match state.embedder.embed(&event.content).await {
            Ok(vector) => {
                sqlx::query("UPDATE chunks SET embedding=$2 WHERE id=$1 AND embedding IS NULL")
                    .bind(chunk)
                    .bind(pgvector::Vector::from(vector))
                    .execute(&state.store.pool)
                    .await?;
            }
            Err(error) => {
                tracing::warn!(chunk_id=%chunk, %error, "source chunk embedding unavailable");
            }
        }
    }
    Ok(())
}

pub async fn run(state: Arc<AppState>, kinds: &[&str]) {
    let mut graph_initialized = !kinds.contains(&"project");
    loop {
        if !graph_initialized {
            if let Some(graph) = &state.graph {
                match graph.ensure_constraints().await {
                    Ok(()) => graph_initialized = true,
                    Err(e) => {
                        tracing::warn!(error=%e, "waiting for Neo4j projection constraints");
                        tokio::time::sleep(Duration::from_secs(5)).await;
                        continue;
                    }
                }
            } else {
                graph_initialized = true;
            }
        }
        match state.store.claim_job(kinds).await {
            Ok(Some(job)) => {
                let mut trace = TraceBuilder::new(
                    &format!("worker_{}", job.kind),
                    &job.namespace,
                    None,
                    json!({"job_id":job.id,"attempt":job.attempts}),
                );
                let start = Instant::now();
                let result = process_with_heartbeat(&state, &job).await;
                trace.step(
                    &job.kind,
                    if result.is_ok() { "ok" } else { "error" },
                    start.elapsed(),
                    match &result {
                        Ok(v) => v.clone(),
                        Err(e) => json!({"error":e.to_string()}),
                    },
                );
                let outcome = result.map_err(|e| e.to_string());
                let completed = trace.finish(
                    if outcome.is_ok() {
                        "succeeded"
                    } else {
                        "failed"
                    },
                    false,
                    outcome.as_ref().ok().cloned(),
                    outcome.as_ref().err().cloned(),
                );
                if let Err(e) = state.store.record_trace(&completed).await {
                    tracing::error!(%e,"worker trace persistence failed");
                }
                if let Err(e) = state.store.finish_job(&job, outcome).await {
                    tracing::error!(%e,"job completion failed");
                }
            }
            Ok(None) => tokio::time::sleep(Duration::from_millis(500)).await,
            Err(e) => {
                tracing::error!(%e,"job claim failed");
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}
async fn process_with_heartbeat(state: &AppState, job: &Job) -> Result<Value, AppError> {
    let work = process(state, job);
    tokio::pin!(work);
    let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            result = &mut work => return result,
            _ = heartbeat.tick() => {
                let mut tx=state.store.pool.begin().await?;
                let updated = sqlx::query("UPDATE memory_jobs SET lease_until=now()+interval '15 minutes' WHERE id=$1 AND lease_token=$2 AND status='running' AND lease_until>now()")
                    .bind(job.id).bind(job.lease_token).execute(&mut *tx).await?;
                if updated.rows_affected() != 1 {
                    return Err(AppError::Conflict("worker lease lost; inference cancelled".into()));
                }
                if ["extract","consolidate"].contains(&job.kind.as_str()) {
                    let renewed=sqlx::query("UPDATE memory_namespace_leases SET lease_until=now()+interval '15 minutes' WHERE job_id=$1 AND lease_token=$2 AND lease_until>now()").bind(job.id).bind(job.lease_token).execute(&mut *tx).await?;
                    if renewed.rows_affected()!=1 { return Err(AppError::Conflict("namespace lease lost; inference cancelled".into())); }
                }
                tx.commit().await?;
            }
        }
    }
}
/// Run one claimed job. Public so a test can run a single job without a claim race.
pub async fn process(state: &AppState, job: &Job) -> Result<Value, AppError> {
    match job.kind.as_str() {
        "extract" => extraction::extract(state, job).await,
        "consolidate" => consolidation::consolidate(state, job).await,
        "project" => projection::project(state, job).await,
        "clear_graph" => projection::clear_graph(state, job).await,
        "rebuild" => projection::rebuild(state, job).await,
        _ => Err(AppError::Validation("unknown job kind".into())),
    }
}
