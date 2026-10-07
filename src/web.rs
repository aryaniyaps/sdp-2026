//! Frontend files are embedded so the release binary needs no asset directory.

use crate::AppState;
use axum::{
    Router,
    extract::Path,
    http::{StatusCode, header},
    response::{Html, IntoResponse},
    routing::get,
};
use std::sync::Arc;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(index))
        .route("/graph", get(index))
        .route("/demo", get(index))
        .route("/assets/{name}", get(asset))
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../frontend/dist/index.html"))
}

async fn asset(Path(name): Path<String>) -> impl IntoResponse {
    let (content_type, contents) = match name.as_str() {
        "app.js" => (
            "text/javascript; charset=utf-8",
            include_str!("../frontend/dist/assets/app.js"),
        ),
        "app.css" => (
            "text/css; charset=utf-8",
            include_str!("../frontend/dist/assets/app.css"),
        ),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        contents,
    )
        .into_response()
}
