use memory_engine::{
    AppState, api,
    graph::GraphStore,
    providers::{OllamaEmbedder, OllamaExtractor},
    store::Store,
};
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc};
use tower_http::trace::TraceLayer;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    dotenvy::dotenv().ok();
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "memory_engine=info,tower_http=info".into());
    if env::var("LOG_FORMAT").as_deref() == Ok("pretty") {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    } else {
        tracing_subscriber::fmt()
            .json()
            .with_current_span(true)
            .with_span_list(true)
            .with_env_filter(filter)
            .init();
    }
    let db = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://memory:memory@127.0.0.1:55432/memory".into());
    let graph_uri = env::var("NEO4J_URI").unwrap_or_else(|_| "http://127.0.0.1:7474".into());
    let graph_user = env::var("NEO4J_USER").unwrap_or_else(|_| "neo4j".into());
    let graph_password = env::var("NEO4J_PASSWORD").unwrap_or_else(|_| "password".into());
    let graph_db = env::var("NEO4J_DATABASE").unwrap_or_else(|_| "neo4j".into());
    let base = env::var("OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let extraction =
        env::var("EXTRACTION_MODEL").unwrap_or_else(|_| "qwen2.5:14b-instruct-q4_K_M".into());
    let embedding = env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "qwen3-embedding:0.6b".into());
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&db)
        .await?;
    let store = Store::new(pool);
    store.migrate().await?;
    let graph = if graph_uri.trim().is_empty() {
        None
    } else {
        let graph_store = GraphStore::new(graph_uri, graph_user, graph_password, Some(graph_db));
        if let Err(err) = graph_store.ensure_constraints().await {
            tracing::warn!(error=%err, "Neo4j constraints not ready; projection worker will retry initialization");
        }
        Some(Arc::new(graph_store))
    };
    let concurrency = env::var("MEMORY_WORKER_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(4)
        .clamp(1, 8);
    let state = Arc::new(AppState {
        store,
        worker_concurrency: concurrency,
        graph,
        extractor: Arc::new(OllamaExtractor::new(base.clone(), extraction)),
        embedder: Arc::new(OllamaEmbedder::new(base, embedding)),
        model: if env::var("MEMORY_MODEL_PROVIDER").as_deref() == Ok("ollama") {
            Arc::new(memory_engine::model::OllamaJsonModel {
                base: env::var("OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into()),
                model: env::var("EXTRACTION_MODEL")
                    .unwrap_or_else(|_| "qwen2.5:14b-instruct-q4_K_M".into()),
            })
        } else {
            Arc::new(memory_engine::model::PiModel {
                executable: env::var("PI_EXECUTABLE").unwrap_or_else(|_| "pi".into()),
                provider: env::var("PI_PROVIDER").unwrap_or_else(|_| "openai".into()),
                model: env::var("PI_MODEL").unwrap_or_else(|_| "gpt-5.6-sol".into()),
            })
        },
        demo_mode: env::var("DEMO_MODE").map(|v| v == "true").unwrap_or(false),
        metrics: Arc::new(memory_engine::observability::Metrics::default()),
    });
    let mut inference_workers = Vec::new();
    for _ in 0..concurrency {
        inference_workers.push(tokio::spawn(memory_engine::worker::run(
            state.clone(),
            &["extract", "consolidate"],
        )));
    }
    let projection_worker = tokio::spawn(memory_engine::worker::run(
        state.clone(),
        &["project", "rebuild", "clear_graph"],
    ));
    let app = api::router(state).layer(TraceLayer::new_for_http().make_span_with(|request:&axum::http::Request<_>|tracing::info_span!("http_request",method=%request.method(),uri=%request.uri())).on_response(|response:&axum::http::Response<_>,latency:std::time::Duration,_span:&tracing::Span|tracing::info!(status=%response.status(),latency_ms=latency.as_millis(),"response completed")));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    tracing::info!("UI http://127.0.0.1:8080 — Swagger http://127.0.0.1:8080/swagger-ui/");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    for worker in inference_workers {
        worker.abort();
    }
    projection_worker.abort();
    Ok(())
}
