pub mod api;
pub mod domain;
pub mod graph;
pub mod knowledge;
pub mod knowledge_store;
pub mod model;
pub mod observability;
pub mod providers;
pub mod search;
pub mod store;
pub mod temporal;
pub mod v2;
pub mod worker;
use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use providers::{DynEmbedder, DynExtractor};
use serde_json::json;
use store::Store;
#[derive(Clone)]
pub struct AppState {
    pub store: Store,
    pub graph: Option<std::sync::Arc<graph::GraphStore>>,
    pub extractor: DynExtractor,
    pub embedder: DynEmbedder,
    pub model: std::sync::Arc<dyn model::JsonModel>,
    pub demo_mode: bool,
    pub worker_concurrency: usize,
    pub metrics: std::sync::Arc<observability::Metrics>,
}
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("validation error: {0}")]
    Validation(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("dependency unavailable: {0}")]
    Unavailable(String),
    #[error("not found")]
    NotFound,
    #[error("demo mode is disabled")]
    Forbidden,
    #[error("database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("migration error: {0}")]
    Migration(#[from] sqlx::migrate::MigrateError),
    #[error("HTTP provider error: {0}")]
    Http(#[from] reqwest::Error),
}
impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::Validation(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Forbidden => StatusCode::FORBIDDEN,
            Self::Provider(_) | Self::Http(_) => StatusCode::BAD_GATEWAY,
            Self::Unavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({"error":self.to_string()}))).into_response()
    }
}
