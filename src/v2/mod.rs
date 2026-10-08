//! HTTP routes for evidence memory. Retrieval and graph logic live in their own modules.

use crate::{
    AppError, AppState,
    graph::{ProjectionNode, ProjectionRelationship},
    knowledge::*,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use uuid::Uuid;

mod dashboard;
mod graph;
mod recall;
mod reflect;

use graph::*;
pub use recall::{
    RecallHit, RecallRequest, RecallResponse, TemporalPlanner, estimate_tokens, recall_engine,
    set_temporal_planner,
};
pub use reflect::reflect_engine;

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        retain,
        recall,
        reflect,
        jobs,
        job,
        retry,
        status,
        assertion,
        retract,
        graph::graph,
        graph::graph_projection,
        graph::graph_memories,
        graph::rebuild,
        graph::clear
    ),
    components(schemas(
        RetainRequest,
        EvidenceEvent,
        EntityInput,
        Cardinality,
        ClaimInput,
        RelatedInput,
        Extraction,
        ObservationInput,
        Consolidation,
        AssertionView,
        EvidenceSource,
        Job,
        RecallRequest,
        RecallResponse,
        RecallHit,
        Namespace,
        ClearRequest,
        ProjectionResponse,
        ProjectionNode,
        ProjectionRelationship,
        ProjectionCounts,
        MemoriesResponse,
        Memory,
        MemorySupport,
        MemoryExtraction,
        MemoryJobCounts,
        MemoriesCounts
    ))
)]
pub struct ApiDoc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route(
            "/api/v2/projects",
            get(dashboard::projects).post(dashboard::create_project),
        )
        .route(
            "/api/v2/dashboard/sessions",
            post(dashboard::create_session),
        )
        .route(
            "/api/v2/dashboard/sessions/{id}",
            get(dashboard::session).post(dashboard::change),
        )
        .route("/api/v2/dashboard/sessions/{id}/ack", post(dashboard::ack))
        .route("/api/v2/retain", post(retain))
        .route("/api/v2/recall", post(recall))
        .route("/api/v2/reflect", post(reflect))
        .route("/api/v2/jobs", get(jobs))
        .route("/api/v2/jobs/{id}", get(job))
        .route("/api/v2/jobs/{id}/retry", post(retry))
        .route("/api/v2/status", get(status))
        .route("/api/v2/assertions/{id}", get(assertion))
        .route("/api/v2/assertions/{id}/retract", post(retract))
        .route("/api/v2/graph", get(graph))
        .route("/api/v2/graph/projection", get(graph_projection))
        .route("/api/v2/graph/memories", get(graph_memories))
        .route("/api/v2/graph/rebuild", post(rebuild))
        .route("/api/v2/graph/clear", post(clear))
}
#[derive(Deserialize, utoipa::ToSchema)]
struct Namespace {
    namespace: String,
}
/// Bounds concurrent early embedding so a bulk import cannot flood the embedder.
static EMBEDDING_GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2);
#[utoipa::path(post,path="/api/v2/retain",responses((status=200,body=Value)),tag="Evidence memory",request_body=RetainRequest)]
async fn retain(
    State(s): State<Arc<AppState>>,
    Json(r): Json<RetainRequest>,
) -> Result<Json<Value>, AppError> {
    let (episode_id, job_id, created) = s.store.retain(&r).await?;
    if created {
        // Make the new evidence semantically searchable now instead of after
        // extraction. Embedding is not on the acknowledgement path: the evidence is
        // already durable, and the extract job fills any gap left by an outage.
        let state = s.clone();
        let namespace = r.namespace.clone();
        tokio::spawn(async move {
            let Ok(_permit) = EMBEDDING_GATE.acquire().await else {
                return;
            };
            let outcome = async {
                let evidence = state.store.episode_evidence(&namespace, episode_id).await?;
                crate::worker::embed_missing_chunks(&state, &evidence).await
            }
            .await;
            if let Err(error) = outcome {
                tracing::warn!(%episode_id, %error, "early source embedding failed");
            }
        });
    }
    Ok(Json(
        json!({"episode_id":episode_id,"job_id":job_id,"created":created}),
    ))
}
#[utoipa::path(get,path="/api/v2/jobs",responses((status=200,body=Vec<Job>)),tag="Evidence memory",params(("namespace"=String,Query)))]
async fn jobs(
    State(s): State<Arc<AppState>>,
    Query(q): Query<Namespace>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(json!(s.store.jobs(&q.namespace).await?)))
}
#[utoipa::path(get,path="/api/v2/jobs/{id}",responses((status=200,body=Job)),tag="Evidence memory",params(("namespace"=String,Query),("id"=Uuid,Path)))]
async fn job(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<Namespace>,
) -> Result<Json<Job>, AppError> {
    Ok(Json(s.store.job(&q.namespace, id).await?))
}
#[utoipa::path(post,path="/api/v2/jobs/{id}/retry",responses((status=200,body=Value)),tag="Evidence memory",params(("namespace"=String,Query),("id"=Uuid,Path)))]
async fn retry(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<Namespace>,
) -> Result<Json<Value>, AppError> {
    s.store.retry_job(&q.namespace, id).await?;
    Ok(Json(json!({"retry":true})))
}
#[utoipa::path(get,path="/api/v2/assertions/{id}",responses((status=200,body=AssertionView)),tag="Evidence memory",params(("namespace"=String,Query),("id"=Uuid,Path)))]
async fn assertion(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<Namespace>,
) -> Result<Json<AssertionView>, AppError> {
    Ok(Json(s.store.assertion(&q.namespace, id).await?))
}
#[utoipa::path(post,path="/api/v2/assertions/{id}/retract",responses((status=200,body=Value)),tag="Evidence memory",params(("namespace"=String,Query),("id"=Uuid,Path)))]
async fn retract(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Query(q): Query<Namespace>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(
        json!({"stale_observations":s.store.retract(&q.namespace,id).await?}),
    ))
}
pub async fn namespace_status(s: &AppState, namespace: &str) -> Result<Value, AppError> {
    let groups:Vec<Value>=sqlx::query_scalar("SELECT jsonb_build_object('kind',kind,'status',status,'count',count(*),'oldest',min(created_at)) FROM memory_jobs WHERE namespace=$1 GROUP BY kind,status ORDER BY kind,status").bind(namespace).fetch_all(&s.store.pool).await?;
    let outstanding:i64=sqlx::query_scalar("SELECT count(*) FROM memory_jobs WHERE namespace=$1 AND status IN ('pending','running','failed')").bind(namespace).fetch_one(&s.store.pool).await?;
    let stale: i64 =
        sqlx::query_scalar("SELECT count(*) FROM assertions WHERE namespace=$1 AND status='stale'")
            .bind(namespace)
            .fetch_one(&s.store.pool)
            .await?;
    let projection:Option<DateTime<Utc>>=sqlx::query_scalar("SELECT max(completed_at) FROM memory_jobs WHERE namespace=$1 AND kind='project' AND status='succeeded'").bind(namespace).fetch_one(&s.store.pool).await?;
    let embedding_gaps: Value = sqlx::query_scalar(
        r#"
SELECT jsonb_build_object('source_chunks',
                            (SELECT count(*)
                             FROM chunks c
                             JOIN raw_events e ON e.id = c.raw_event_id
                             JOIN sessions se ON se.id = e.session_id
                             WHERE se.namespace = $1
                               AND c.embedding IS NULL),'active_assertions',
                            (SELECT count(*)
                             FROM assertions
                             WHERE namespace = $1
                               AND status IN ('active', 'contested')
                               AND embedding IS NULL))
"#,
    )
    .bind(namespace)
    .fetch_one(&s.store.pool)
    .await?;
    let worker_operations:Vec<Value>=sqlx::query_scalar(r#"
SELECT jsonb_build_object('operation', operation_type, 'attempts', count(*), 'failed_attempts', count(*) FILTER(
                                                                                                                WHERE status = 'failed'), 'total_ms', sum(duration_ms), 'mean_ms', avg(duration_ms))
FROM operations
WHERE namespace = $1
  AND operation_type LIKE 'worker_%'
GROUP BY operation_type
ORDER BY operation_type
"#).bind(namespace).fetch_all(&s.store.pool).await?;
    Ok(
        json!({"namespace":namespace,"ready":outstanding==0,"outstanding_jobs":outstanding,"jobs":groups,"stale_observations":stale,"last_projection_completed_at":projection,"embedding_gaps":embedding_gaps,"worker_operations":worker_operations}),
    )
}
#[utoipa::path(get,path="/api/v2/status",responses((status=200,body=Value)),tag="Evidence memory",params(("namespace"=String,Query)))]
async fn status(
    State(s): State<Arc<AppState>>,
    Query(q): Query<Namespace>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(namespace_status(&s, &q.namespace).await?))
}
#[utoipa::path(post,path="/api/v2/recall",responses((status=200,body=RecallResponse)),tag="Evidence memory",request_body=RecallRequest)]
async fn recall(
    State(s): State<Arc<AppState>>,
    Json(r): Json<RecallRequest>,
) -> Result<Json<RecallResponse>, AppError> {
    Ok(Json(recall_engine(&s, r).await?))
}
#[utoipa::path(post,path="/api/v2/reflect",responses((status=200,body=Value)),tag="Evidence memory",request_body=RecallRequest)]
async fn reflect(
    State(s): State<Arc<AppState>>,
    Json(r): Json<RecallRequest>,
) -> Result<Json<Value>, AppError> {
    Ok(Json(reflect_engine(&s, r).await?))
}
