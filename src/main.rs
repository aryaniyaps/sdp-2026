use memory_engine::{
    AppState, api,
    providers::{OllamaEmbedder, OllamaExtractor},
    store::Store,
};
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc};
use tower_http::trace::TraceLayer;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    dotenvy::dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "memory_engine=info,tower_http=info".into()),
        )
        .init();
    let db = env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://memory:memory@127.0.0.1:55432/memory".into());
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
    let state = Arc::new(AppState {
        store,
        extractor: Arc::new(OllamaExtractor::new(base.clone(), extraction)),
        embedder: Arc::new(OllamaEmbedder::new(base, embedding)),
        demo_mode: env::var("DEMO_MODE").map(|v| v == "true").unwrap_or(false),
    });
    let app = api::router(state).layer(TraceLayer::new_for_http());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
    tracing::info!("UI http://127.0.0.1:8080 — Swagger http://127.0.0.1:8080/swagger-ui/");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(())
}
