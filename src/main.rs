use memory_engine::{
    AppState, api,
    providers::{AzureEmbedder, AzureExtractor, DynEmbedder, DynExtractor, OllamaEmbedder,
                OllamaExtractor},
    store::Store,
};
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc};
use tower_http::trace::TraceLayer;
#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let base = env::var("OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into());
    let extraction =
        env::var("EXTRACTION_MODEL").unwrap_or_else(|_| "qwen2.5:14b-instruct-q4_K_M".into());
    let embedding = env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "qwen3-embedding:0.6b".into());
    // Provider selection. Ollama stays the default so local review is unchanged;
    // MODEL_PROVIDER=azure is what makes CPU-only container hosting possible.
    let (extractor, embedder): (DynExtractor, DynEmbedder) =
        if env::var("MODEL_PROVIDER").as_deref() == Ok("azure") {
            let endpoint = env::var("AZURE_OPENAI_ENDPOINT")
                .expect("AZURE_OPENAI_ENDPOINT required when MODEL_PROVIDER=azure");
            let key = env::var("AZURE_OPENAI_KEY")
                .expect("AZURE_OPENAI_KEY required when MODEL_PROVIDER=azure");
            let embed_dep = env::var("AZURE_EMBEDDING_DEPLOYMENT")
                .unwrap_or_else(|_| "text-embedding-3-small".into());
            let extract_dep = env::var("AZURE_EXTRACTION_DEPLOYMENT")
                .unwrap_or_else(|_| "gpt-5.6-sol".into());
            tracing::info!(embed = %embed_dep, extract = %extract_dep, "using azure providers");
            (
                Arc::new(AzureExtractor::new(endpoint.clone(), key.clone(), extract_dep)),
                Arc::new(AzureEmbedder::new(endpoint, key, embed_dep)),
            )
        } else {
            (
                Arc::new(OllamaExtractor::new(base.clone(), extraction)),
                Arc::new(OllamaEmbedder::new(base, embedding)),
            )
        };

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&db)
        .await?;
    let store = Store::new(pool);
    store.migrate().await?;
    let state = Arc::new(AppState {
        store,
        extractor,
        embedder,
        demo_mode: env::var("DEMO_MODE").map(|v| v == "true").unwrap_or(false),
        metrics: Arc::new(memory_engine::observability::Metrics::default()),
    });
    let app = api::router(state).layer(TraceLayer::new_for_http().make_span_with(|request:&axum::http::Request<_>|tracing::info_span!("http_request",method=%request.method(),uri=%request.uri())).on_response(|response:&axum::http::Response<_>,latency:std::time::Duration,_span:&tracing::Span|tracing::info!(status=%response.status(),latency_ms=latency.as_millis(),"response completed")));
    let port = env::var("PORT").unwrap_or_else(|_| "8080".into());
    let bind = env::var("BIND_ADDR").unwrap_or_else(|_| "127.0.0.1".into());
    let listener = tokio::net::TcpListener::bind(format!("{bind}:{port}")).await?;
    tracing::info!("UI http://{bind}:{port} — Swagger http://{bind}:{port}/swagger-ui/");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            tokio::signal::ctrl_c().await.ok();
        })
        .await?;
    Ok(())
}
