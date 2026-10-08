use memory_engine::{
    AppState, api,
    graph::GraphStore,
    model::{OllamaJsonModel, OllamaLimits},
    providers::{self, OllamaEmbedder},
    store::Store,
};
use sqlx::postgres::PgPoolOptions;
use std::{env, sync::Arc};
use tower_http::{
    LatencyUnit,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};
use tracing::Level;

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
    let embedding = env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "qwen3-embedding:0.6b".into());
    // The worker is the fine-tuned student served by Ollama. Pi and its login are for the coding
    // session only. A leftover setting for another provider must not be ignored quietly.
    match env::var("MEMORY_MODEL_PROVIDER") {
        Ok(provider) if provider != "ollama" => {
            return Err(format!(
                "MEMORY_MODEL_PROVIDER={provider} is no longer supported: the worker is always the local model served by Ollama (EXTRACTION_MODEL at EXTRACTION_OLLAMA_URL). Unset MEMORY_MODEL_PROVIDER."
            )
            .into());
        }
        Ok(_) | Err(env::VarError::NotPresent) => {}
        Err(err) => return Err(format!("MEMORY_MODEL_PROVIDER is not usable: {err}").into()),
    }
    // The model may be served from another machine than the embedder (a GPU workstation).
    let model_base = env::var("EXTRACTION_OLLAMA_URL").unwrap_or_else(|_| base.clone());
    let extraction_model = env::var("EXTRACTION_MODEL").unwrap_or_else(|_| "mem-extractor".into());
    let ollama_limits = OllamaLimits::from_env()?;
    let planner = memory_engine::v2::TemporalPlanner::from_env()?;
    memory_engine::v2::set_temporal_planner(planner);
    tracing::info!(?planner, "temporal planner");
    tracing::info!(
        num_ctx = ollama_limits.num_ctx,
        num_predict = ollama_limits.num_predict,
        "Ollama context limits"
    );
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
        embedder: Arc::new(
            OllamaEmbedder::new(base.clone(), embedding).with_keep_alive(
                env::var("OLLAMA_KEEP_ALIVE")
                    .unwrap_or_else(|_| providers::DEFAULT_KEEP_ALIVE.into()),
            ),
        ),
        model: Arc::new(OllamaJsonModel {
            base: model_base.clone(),
            model: extraction_model.clone(),
            limits: ollama_limits,
        }),
    });
    let bind_addr = match env::var("BIND_ADDR") {
        Ok(addr) => addr,
        Err(env::VarError::NotPresent) => "127.0.0.1:8080".into(),
        Err(err) => return Err(format!("BIND_ADDR is not usable: {err}").into()),
    };
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|err| format!("cannot listen on {bind_addr}: {err}"))?;
    // Say at start-up, not at the first failed job, when the worker model is not served.
    let (probe_base, probe_model) = (model_base.clone(), extraction_model.clone());
    tokio::spawn(async move {
        let shown = reqwest::Client::new()
            .post(format!("{probe_base}/api/show"))
            .timeout(std::time::Duration::from_secs(10))
            .json(&serde_json::json!({"model": probe_model}))
            .send()
            .await;
        match shown {
            Ok(response) if response.status().is_success() => {
                tracing::info!(model=%probe_model, base=%probe_base, "worker model is served")
            }
            Ok(response) => tracing::error!(
                model=%probe_model, base=%probe_base, status=%response.status(),
                "worker model is not available; extraction jobs will fail until it is. Run scripts/fetch-slm.sh"
            ),
            Err(err) => tracing::error!(
                model=%probe_model, base=%probe_base, error=%err,
                "worker model server is not reachable; extraction jobs will fail until it is"
            ),
        }
    });
    // Load the embedding model now, so the first recall after a start does not pay for it.
    let warm = state.embedder.clone();
    tokio::spawn(async move {
        match warm.embed("warm up").await {
            Ok(_) => tracing::info!("embedding model loaded"),
            Err(err) => {
                tracing::warn!(error=%err, "embedding warm-up failed; the first recall will load the model")
            }
        }
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
    let trace_layer = TraceLayer::new_for_http()
        .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
        .on_response(
            DefaultOnResponse::new()
                .level(Level::INFO)
                .latency_unit(LatencyUnit::Millis),
        );
    let app = api::router(state).layer(trace_layer);
    tracing::info!("UI http://{bind_addr} - Swagger http://{bind_addr}/swagger-ui/");
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
