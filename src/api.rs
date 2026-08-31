use crate::{
    AppError, AppState,
    domain::{ExtractedMemory, MemoryKind, ResolveOutcome, VersionView},
    observability::{OperationTrace, TraceBuilder},
    search,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    response::{Html, IntoResponse},
    routing::{get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{sync::Arc, time::Instant};
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;
use uuid::Uuid;

#[derive(OpenApi)]
#[openapi(
    paths(
        health,
        create_session,
        ingest,
        search_memories,
        list_memories,
        memory_chain,
        list_traces,
        get_trace,
        reset_demo
    ),
    components(schemas(
        SessionRequest,
        SessionResponse,
        EventRequest,
        EventResponse,
        SearchRequest,
        SearchResponse,
        HealthResponse,
        MemoriesQuery,
        VersionView,
        ResolveOutcome,
        OperationTrace,
        crate::observability::OperationStep,
        crate::search::RankedCandidate
    ))
)]
pub struct ApiDoc;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/healthz", get(health))
        .route("/api/v1/sessions", post(create_session))
        .route("/api/v1/events", post(ingest))
        .route("/api/v1/search", post(search_memories))
        .route("/api/v1/memories", get(list_memories))
        .route("/api/v1/memories/{id}", get(memory_chain))
        .route("/api/v1/traces", get(list_traces))
        .route("/api/v1/traces/{id}", get(get_trace))
        .route("/metrics", get(metrics))
        .route("/api/v1/demo/reset", post(reset_demo))
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .with_state(state)
}

#[derive(Deserialize, ToSchema)]
pub struct SessionRequest {
    pub namespace: String,
    pub external_id: String,
}
#[derive(Serialize, ToSchema)]
pub struct SessionResponse {
    pub id: Uuid,
    pub created: bool,
}
#[utoipa::path(post,path="/api/v1/sessions",request_body=SessionRequest,responses((status=200,body=SessionResponse)))]
async fn create_session(
    State(s): State<Arc<AppState>>,
    Json(r): Json<SessionRequest>,
) -> Result<Json<SessionResponse>, AppError> {
    required(&r.namespace, "namespace")?;
    required(&r.external_id, "external_id")?;
    let (id, created) = s
        .store
        .create_session(r.namespace.trim(), r.external_id.trim())
        .await?;
    Ok(Json(SessionResponse { id, created }))
}

#[derive(Deserialize, ToSchema)]
pub struct EventRequest {
    pub session_id: Uuid,
    pub role: String,
    pub content: String,
    pub occurred_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub deterministic_fixture: bool,
}
#[derive(Serialize, ToSchema)]
pub struct EventResponse {
    pub trace_id: Uuid,
    pub event_id: Uuid,
    pub outcomes: Vec<ResolveOutcome>,
    pub degraded_mode: bool,
    pub elapsed_ms: u128,
}
#[utoipa::path(post,path="/api/v1/events",request_body=EventRequest,responses((status=200,body=EventResponse),(status=409)))]
async fn ingest(
    State(s): State<Arc<AppState>>,
    Json(r): Json<EventRequest>,
) -> Result<Json<EventResponse>, AppError> {
    required(&r.content, "content")?;
    if !["user", "assistant", "system"].contains(&r.role.as_str()) {
        return Err(AppError::Validation(
            "role must be user, assistant, or system".into(),
        ));
    }
    let started = Instant::now();
    let at = r.occurred_at.unwrap_or_else(Utc::now);
    let namespace: Option<String> =
        sqlx::query_scalar("SELECT namespace FROM sessions WHERE id=$1")
            .bind(r.session_id)
            .fetch_optional(&s.store.pool)
            .await?;
    let namespace = namespace.ok_or(AppError::NotFound)?;
    let mut trace = TraceBuilder::new(
        "ingest",
        &namespace,
        Some(r.session_id),
        json!({"role":r.role,"content_characters":r.content.chars().count(),"occurred_at":at,"deterministic_fixture":r.deterministic_fixture}),
    );
    trace.step(
        "validate_request",
        "ok",
        started.elapsed(),
        json!({"namespace":namespace,"role":r.role,"content_characters":r.content.chars().count()}),
    );
    let evidence_started = Instant::now();
    let (event, chunk) = s
        .store
        .begin_event(r.session_id, &r.role, &r.content, at)
        .await?;
    trace.set_event(event);
    trace.step(
        "persist_immutable_evidence",
        "ok",
        evidence_started.elapsed(),
        json!({"event_id":event,"chunk_id":chunk,"processing_state":"pending"}),
    );
    let extraction_started = Instant::now();
    let memories = if r.deterministic_fixture {
        if !s.demo_mode {
            return Err(AppError::Forbidden);
        }
        let items = fixture_extract(&r.content);
        trace.step("extract_typed_memories", "ok", extraction_started.elapsed(), json!({"provider":"deterministic_fixture","model":s.extractor.version(),"temperature":0,"memory_count":items.len(),"memories":items}));
        items
    } else {
        match s.extractor.extract(&r.content).await {
            Ok(m) => {
                trace.step("extract_typed_memories", "ok", extraction_started.elapsed(), json!({"provider":"ollama","model":s.extractor.version(),"temperature":0,"schema_constrained":true,"memory_count":m.len(),"memories":m}));
                m
            }
            Err(e) => {
                trace.step("extract_typed_memories", "error", extraction_started.elapsed(), json!({"provider":"ollama","model":s.extractor.version(),"error":e.to_string()}));
                s.store.fail_event(event, &e.to_string()).await?;
                let completed = trace.finish("failed", false, None, Some(e.to_string()));
                s.metrics.inc(&s.metrics.failures, 1);
                let _ = s.store.record_trace(&completed).await;
                return Err(e);
            }
        }
    };
    let mut outcomes = Vec::new();
    let mut degraded = false;
    for m in memories {
        let embedding_started = Instant::now();
        let embedding = match s.embedder.embed(&m.statement).await {
            Ok(v) => {
                trace.step("embed_memory", "ok", embedding_started.elapsed(), json!({"model":s.embedder.version(),"dimensions":v.len(),"canonical_key":crate::domain::canonical_key(&m.subject,&m.predicate)}));
                Some(v)
            }
            Err(e) => {
                degraded = true;
                trace.step("embed_memory", "degraded", embedding_started.elapsed(), json!({"model":s.embedder.version(),"fallback":"store_without_vector","error":e.to_string()}));
                None
            }
        };
        let resolution_started = Instant::now();
        match s
            .store
            .resolve(&namespace, &m, chunk, at, s.extractor.version(), embedding)
            .await
        {
            Ok(x) => {
                trace.step("resolve_version_transaction", "ok", resolution_started.elapsed(), json!({"canonical_key":crate::domain::canonical_key(&m.subject,&m.predicate),"normalized_value":crate::domain::normalize_component(&m.value),"decision":x.action,"memory_id":x.memory_id,"version_id":x.version_id,"version":x.version}));
                outcomes.push(x)
            }
            Err(e) => {
                trace.step(
                    "resolve_version_transaction",
                    "error",
                    resolution_started.elapsed(),
                    json!({"error":e.to_string()}),
                );
                s.store.fail_event(event, &e.to_string()).await?;
                let completed = trace.finish("failed", degraded, None, Some(e.to_string()));
                s.metrics.inc(&s.metrics.failures, 1);
                let _ = s.store.record_trace(&completed).await;
                return Err(e);
            }
        }
    }
    let finalize_started = Instant::now();
    s.store.complete_event(event).await?;
    trace.step(
        "finalize_event",
        "ok",
        finalize_started.elapsed(),
        json!({"processing_state":"processed"}),
    );
    let elapsed = started.elapsed().as_millis();
    let trace_id = trace.id();
    let result = json!({"event_id":event,"outcomes":outcomes,"degraded_mode":degraded});
    let completed = trace.finish(
        if degraded { "degraded" } else { "succeeded" },
        degraded,
        Some(result),
        None,
    );
    s.store.record_trace(&completed).await?;
    s.metrics.inc(&s.metrics.ingestions, 1);
    s.metrics
        .inc(&s.metrics.extracted_memories, outcomes.len() as u64);
    s.metrics.inc(&s.metrics.ingestion_ms, elapsed as u64);
    if degraded {
        s.metrics.inc(&s.metrics.degraded, 1)
    }
    for o in &outcomes {
        match o.action.as_str() {
            "created" => s.metrics.inc(&s.metrics.versions_created, 1),
            "reinforced" => s.metrics.inc(&s.metrics.versions_reinforced, 1),
            "superseded" => s.metrics.inc(&s.metrics.versions_superseded, 1),
            _ => {}
        }
    }
    tracing::info!(operation_id=%trace_id,event_id=%event,namespace=%namespace,elapsed_ms=elapsed,degraded,memories=outcomes.len(),"ingestion completed");
    Ok(Json(EventResponse {
        trace_id,
        event_id: event,
        outcomes,
        degraded_mode: degraded,
        elapsed_ms: elapsed,
    }))
}
fn fixture_extract(text: &str) -> Vec<ExtractedMemory> {
    let lower = text.to_lowercase();
    let value = if lower.contains("rust") {
        "Rust"
    } else if lower.contains("python") {
        "Python"
    } else {
        return vec![];
    };
    vec![ExtractedMemory {
        subject: "Aryan".into(),
        predicate: "preferred programming language".into(),
        value: value.into(),
        statement: format!("Aryan prefers {value} for programming."),
        kind: MemoryKind::Preference,
    }]
}

#[derive(Deserialize, ToSchema)]
pub struct SearchRequest {
    pub namespace: String,
    pub query: String,
    pub session_id: Option<Uuid>,
    pub top_k: Option<usize>,
    pub max_tokens: Option<usize>,
    #[serde(default)]
    pub include_history: bool,
}
#[derive(Serialize, ToSchema)]
pub struct SearchResponse {
    pub trace_id: Uuid,
    pub results: Vec<crate::search::RankedCandidate>,
    pub context: String,
    pub estimated_tokens: usize,
    pub degraded_mode: bool,
    pub elapsed_ms: u128,
}
#[utoipa::path(post,path="/api/v1/search",request_body=SearchRequest,responses((status=200,body=SearchResponse)))]
async fn search_memories(
    State(s): State<Arc<AppState>>,
    Json(r): Json<SearchRequest>,
) -> Result<Json<SearchResponse>, AppError> {
    required(&r.namespace, "namespace")?;
    required(&r.query, "query")?;
    let started = Instant::now();
    let top = r.top_k.unwrap_or(5).clamp(1, 20);
    let budget = r.max_tokens.unwrap_or(256).clamp(1, 8192);
    let mut trace = TraceBuilder::new(
        "search",
        &r.namespace,
        r.session_id,
        json!({"query":r.query,"top_k":top,"max_tokens":budget,"include_history":r.include_history,"session_id":r.session_id}),
    );
    trace.step("validate_and_plan", "ok", started.elapsed(), json!({"lexical_limit":20,"semantic_limit":20,"rrf_k":60,"top_k":top,"token_budget":budget,"active_only":!r.include_history}));
    let lexical_started = Instant::now();
    let lexical = s
        .store
        .lexical(&r.namespace, &r.query, r.session_id, r.include_history)
        .await?;
    trace.step("lexical_retrieval", "ok", lexical_started.elapsed(), json!({"engine":"PostgreSQL websearch_to_tsquery + GIN","candidate_count":lexical.len(),"candidates":lexical.iter().enumerate().map(|(i,(id,statement))|json!({"rank":i+1,"version_id":id,"statement":statement})).collect::<Vec<_>>()}));
    let embed_started = Instant::now();
    let (semantic, degraded) = match s.embedder.embed(&r.query).await {
        Ok(e) => {
            trace.step(
                "embed_query",
                "ok",
                embed_started.elapsed(),
                json!({"model":s.embedder.version(),"dimensions":e.len()}),
            );
            let semantic_started = Instant::now();
            let rows = s
                .store
                .semantic(&r.namespace, e, r.session_id, r.include_history)
                .await?;
            trace.step("semantic_retrieval", "ok", semantic_started.elapsed(), json!({"engine":"pgvector HNSW cosine distance","candidate_count":rows.len(),"candidates":rows.iter().enumerate().map(|(i,(id,statement))|json!({"rank":i+1,"version_id":id,"statement":statement})).collect::<Vec<_>>()}));
            (rows, false)
        }
        Err(e) => {
            trace.step("embed_query", "degraded", embed_started.elapsed(), json!({"model":s.embedder.version(),"fallback":"lexical_only","error":e.to_string()}));
            trace.step(
                "semantic_retrieval",
                "skipped",
                std::time::Duration::ZERO,
                json!({"reason":"query embedding unavailable"}),
            );
            (vec![], true)
        }
    };
    let fusion_started = Instant::now();
    let fused = search::reciprocal_rank_fusion(&lexical, &semantic, 60.0);
    trace.step(
        "reciprocal_rank_fusion",
        "ok",
        fusion_started.elapsed(),
        json!({"formula":"score += 1 / (60 + rank)","candidate_count":fused.len(),"ranking":fused}),
    );
    let packing_started = Instant::now();
    let (results, context, estimated_tokens) = search::pack(&fused, budget, top);
    trace.step("token_budget_packing", "ok", packing_started.elapsed(), json!({"estimator":"ceil(Unicode characters / 4)","budget":budget,"estimated_tokens":estimated_tokens,"included_version_ids":results.iter().map(|x|x.version_id).collect::<Vec<_>>(),"context":context}));
    let elapsed = started.elapsed().as_millis();
    let trace_id = trace.id();
    let completed=trace.finish(if degraded{"degraded"}else{"succeeded"},degraded,Some(json!({"result_count":results.len(),"estimated_tokens":estimated_tokens,"degraded_mode":degraded})),None);
    s.store.record_trace(&completed).await?;
    s.metrics.inc(&s.metrics.searches, 1);
    s.metrics.inc(&s.metrics.search_ms, elapsed as u64);
    if degraded {
        s.metrics.inc(&s.metrics.degraded, 1)
    }
    tracing::info!(operation_id=%trace_id,namespace=%r.namespace,elapsed_ms=elapsed,degraded,results=results.len(),estimated_tokens,"search completed");
    Ok(Json(SearchResponse {
        trace_id,
        results,
        context,
        estimated_tokens,
        degraded_mode: degraded,
        elapsed_ms: elapsed,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct MemoriesQuery {
    pub namespace: String,
    #[serde(default)]
    pub include_history: bool,
}
#[utoipa::path(get,path="/api/v1/memories",params(("namespace"=String,Query),("include_history"=bool,Query)),responses((status=200,body=[VersionView])))]
async fn list_memories(
    State(s): State<Arc<AppState>>,
    Query(q): Query<MemoriesQuery>,
) -> Result<Json<Vec<VersionView>>, AppError> {
    Ok(Json(s.store.list(&q.namespace, q.include_history).await?))
}
#[utoipa::path(get,path="/api/v1/memories/{id}",params(("id"=Uuid,Path)),responses((status=200,body=[VersionView]),(status=404)))]
async fn memory_chain(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<VersionView>>, AppError> {
    Ok(Json(s.store.chain(id).await?))
}

#[derive(Deserialize)]
struct TracesQuery {
    namespace: String,
    limit: Option<i64>,
}
#[utoipa::path(get,path="/api/v1/traces",params(("namespace"=String,Query),("limit"=Option<i64>,Query)),responses((status=200,body=[OperationTrace])))]
async fn list_traces(
    State(s): State<Arc<AppState>>,
    Query(q): Query<TracesQuery>,
) -> Result<Json<Vec<OperationTrace>>, AppError> {
    required(&q.namespace, "namespace")?;
    Ok(Json(
        s.store.traces(&q.namespace, q.limit.unwrap_or(20)).await?,
    ))
}
#[utoipa::path(get,path="/api/v1/traces/{id}",params(("id"=Uuid,Path)),responses((status=200,body=OperationTrace),(status=404)))]
async fn get_trace(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<OperationTrace>, AppError> {
    Ok(Json(s.store.trace(id).await?))
}
async fn metrics(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    (
        [("content-type", "text/plain; version=0.0.4")],
        s.metrics.prometheus(),
    )
}

#[derive(Serialize, ToSchema)]
pub struct HealthResponse {
    pub database: bool,
    pub extractor: bool,
    pub embedder: bool,
    pub degraded_mode: bool,
}
#[utoipa::path(get,path="/healthz",responses((status=200,body=HealthResponse)))]
async fn health(State(s): State<Arc<AppState>>) -> Json<HealthResponse> {
    let (database, extractor, embedder) = tokio::join!(
        async { sqlx::query("SELECT 1").execute(&s.store.pool).await.is_ok() },
        s.extractor.ready(),
        s.embedder.ready()
    );
    Json(HealthResponse {
        database,
        extractor,
        embedder,
        degraded_mode: !extractor || !embedder,
    })
}
#[utoipa::path(post,path="/api/v1/demo/reset",responses((status=200)))]
async fn reset_demo(State(s): State<Arc<AppState>>) -> Result<impl IntoResponse, AppError> {
    if !s.demo_mode {
        return Err(AppError::Forbidden);
    }
    s.store.reset().await?;
    Ok(Json(serde_json::json!({"reset":true})))
}
async fn index() -> Html<&'static str> {
    Html(include_str!("ui.html"))
}
fn required(v: &str, name: &str) -> Result<(), AppError> {
    if v.trim().is_empty() {
        Err(AppError::Validation(format!("{name} is required")))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn embedded_ui_has_one_observatory_and_one_ledger() {
        let ui = include_str!("ui.html");
        assert_eq!(ui.matches("Operation observatory").count(), 1);
        assert_eq!(ui.matches("Memory ledger and provenance").count(), 1);
        assert_eq!(ui.matches("id=\"memories\"").count(), 1);
        assert_eq!(ui.matches("<script>").count(), 1);
        assert_eq!(ui.matches("</main>").count(), 1);
    }
}
