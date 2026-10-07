//! Shared service routes: health, metrics, traces, and API documentation.

use crate::{AppError, AppState, observability::OperationTrace};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::IntoResponse,
    routing::get,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

#[derive(OpenApi)]
#[openapi(
    paths(health, list_traces, get_trace),
    components(schemas(HealthResponse, OperationTrace, crate::observability::OperationStep))
)]
pub struct ApiDoc;

pub fn router(state: Arc<AppState>) -> Router {
    let mut document = ApiDoc::openapi();
    document.merge(crate::v2::ApiDoc::openapi());
    Router::new()
        .route("/healthz", get(health))
        .route("/metrics", get(metrics))
        .route("/api/v2/traces", get(list_traces))
        .route("/api/v2/traces/{id}", get(get_trace))
        .merge(crate::web::router())
        .merge(crate::v2::router())
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", document))
        .with_state(state)
}

#[derive(Deserialize)]
struct TracesQuery {
    namespace: String,
    limit: Option<i64>,
}
#[utoipa::path(get,path="/api/v2/traces",params(("namespace"=String,Query),("limit"=Option<i64>,Query)),responses((status=200,body=[OperationTrace])))]
async fn list_traces(
    State(s): State<Arc<AppState>>,
    Query(q): Query<TracesQuery>,
) -> Result<Json<Vec<OperationTrace>>, AppError> {
    required(&q.namespace, "namespace")?;
    Ok(Json(
        s.store.traces(&q.namespace, q.limit.unwrap_or(20)).await?,
    ))
}
#[utoipa::path(get,path="/api/v2/traces/{id}",params(("id"=Uuid,Path)),responses((status=200,body=OperationTrace),(status=404)))]
async fn get_trace(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<OperationTrace>, AppError> {
    Ok(Json(s.store.trace(id).await?))
}
async fn metrics(State(s): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    // Use durable traces rather than counters that disappear when the service restarts.
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT operation_type, status, count(*), coalesce(sum(duration_ms), 0)::bigint FROM operations GROUP BY operation_type, status ORDER BY operation_type, status",
    )
    .fetch_all(&s.store.pool)
    .await?;
    let mut output = String::from(
        "# HELP memory_engine_operations_total Recorded operations by type and status.\n\
         # TYPE memory_engine_operations_total counter\n\
         # HELP memory_engine_operation_duration_ms_sum Total processing time in milliseconds.\n\
         # TYPE memory_engine_operation_duration_ms_sum counter\n",
    );
    for (operation, status, count, duration) in rows {
        let labels = format!(
            "operation={},status={}",
            serde_json::to_string(&operation).unwrap(),
            serde_json::to_string(&status).unwrap()
        );
        output.push_str(&format!(
            "memory_engine_operations_total{{{labels}}} {count}\n"
        ));
        output.push_str(&format!(
            "memory_engine_operation_duration_ms_sum{{{labels}}} {duration}\n"
        ));
    }
    Ok(([("content-type", "text/plain; version=0.0.4")], output))
}

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub database: bool,
    pub embedder: bool,
    pub degraded_mode: bool,
    pub worker_model: String,
    /// Context window and output cap of a local worker model; absent for hosted models.
    pub worker_num_ctx: Option<usize>,
    pub worker_num_predict: Option<usize>,
    pub worker_concurrency: usize,
    pub embedding_model: String,
}
#[utoipa::path(get,path="/healthz",responses((status=200,body=HealthResponse)))]
async fn health(State(s): State<Arc<AppState>>) -> Json<HealthResponse> {
    let (database, embedder) = tokio::join!(
        async { sqlx::query("SELECT 1").execute(&s.store.pool).await.is_ok() },
        s.embedder.ready()
    );
    Json(HealthResponse {
        database,
        embedder,
        degraded_mode: !database || !embedder,
        worker_model: s.model.identity(),
        worker_num_ctx: s.model.context_limits().map(|l| l.num_ctx),
        worker_num_predict: s.model.context_limits().map(|l| l.num_predict),
        worker_concurrency: s.worker_concurrency,
        embedding_model: s.embedder.version().into(),
    })
}
fn required(v: &str, name: &str) -> Result<(), AppError> {
    if v.trim().is_empty() {
        Err(AppError::Validation(format!("{name} is required")))
    } else {
        Ok(())
    }
}
