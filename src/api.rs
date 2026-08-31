use crate::{
    AppError, AppState,
    domain::{ExtractedMemory, MemoryKind, ResolveOutcome, VersionView},
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
    let (event, chunk) = s
        .store
        .begin_event(r.session_id, &r.role, &r.content, at)
        .await?;
    let memories = if r.deterministic_fixture {
        if !s.demo_mode {
            return Err(AppError::Forbidden);
        }
        fixture_extract(&r.content)
    } else {
        match s.extractor.extract(&r.content).await {
            Ok(m) => m,
            Err(e) => {
                s.store.fail_event(event, &e.to_string()).await?;
                return Err(e);
            }
        }
    };
    let mut outcomes = Vec::new();
    let mut degraded = false;
    for m in memories {
        let embedding = match s.embedder.embed(&m.statement).await {
            Ok(v) => Some(v),
            Err(_) => {
                degraded = true;
                None
            }
        };
        match s
            .store
            .resolve(&namespace, &m, chunk, at, s.extractor.version(), embedding)
            .await
        {
            Ok(x) => outcomes.push(x),
            Err(e) => {
                s.store.fail_event(event, &e.to_string()).await?;
                return Err(e);
            }
        }
    }
    s.store.complete_event(event).await?;
    Ok(Json(EventResponse {
        event_id: event,
        outcomes,
        degraded_mode: degraded,
        elapsed_ms: started.elapsed().as_millis(),
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
    let lexical = s
        .store
        .lexical(&r.namespace, &r.query, r.session_id, r.include_history)
        .await?;
    let (semantic, degraded) = match s.embedder.embed(&r.query).await {
        Ok(e) => (
            s.store
                .semantic(&r.namespace, e, r.session_id, r.include_history)
                .await?,
            false,
        ),
        Err(_) => (vec![], true),
    };
    let fused = search::reciprocal_rank_fusion(&lexical, &semantic, 60.0);
    let (results, context, estimated_tokens) = search::pack(&fused, budget, top);
    Ok(Json(SearchResponse {
        results,
        context,
        estimated_tokens,
        degraded_mode: degraded,
        elapsed_ms: started.elapsed().as_millis(),
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
