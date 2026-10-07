use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use memory_engine::{
    AppError, AppState, api,
    domain::{ExtractedMemory, MemoryKind},
    providers::{Embedder, Extractor},
    store::Store,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower::ServiceExt;

struct FakeExtractor;
#[async_trait]
impl Extractor for FakeExtractor {
    async fn extract(&self, text: &str) -> Result<Vec<ExtractedMemory>, AppError> {
        if text == "FAIL" {
            return Err(AppError::Provider("fixture failure".into()));
        }
        let value = if text.to_lowercase().contains("rust") {
            "Rust"
        } else {
            "Python"
        };
        Ok(vec![ExtractedMemory {
            subject: "Aryan".into(),
            predicate: "preferred programming language".into(),
            value: value.into(),
            statement: format!("Aryan prefers {value} for programming."),
            kind: MemoryKind::Preference,
        }])
    }
    async fn ready(&self) -> bool {
        true
    }
    fn version(&self) -> &str {
        "fake-v1"
    }
}
struct FakeEmbedder {
    fail: bool,
}
#[async_trait]
impl Embedder for FakeEmbedder {
    async fn embed(&self, _: &str) -> Result<Vec<f32>, AppError> {
        if self.fail {
            Err(AppError::Provider("offline".into()))
        } else {
            let mut v = vec![0.; 1024];
            v[0] = 1.;
            Ok(v)
        }
    }
    async fn ready(&self) -> bool {
        !self.fail
    }
    fn version(&self) -> &str {
        "fake-embedding-v1"
    }
}

async fn call(app: &axum::Router, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
async fn postgres_versioning_provenance_isolation_and_api_contracts() {
    let url = match std::env::var("TEST_DATABASE_URL").or_else(|_| std::env::var("DATABASE_URL")) {
        Ok(x) => x,
        Err(_) => {
            eprintln!("skipping: set TEST_DATABASE_URL for PostgreSQL integration test");
            return;
        }
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .unwrap();
    let store = Store::new(pool);
    store.migrate().await.unwrap();
    store.reset().await.unwrap();
    let state = Arc::new(AppState {
        store: store.clone(),
        graph: None,
        extractor: Arc::new(FakeExtractor),
        embedder: Arc::new(FakeEmbedder { fail: false }),
        model: Arc::new(memory_engine::model::OllamaJsonModel {
            base: "http://127.0.0.1:1".into(),
            model: "unused".into(),
            limits: memory_engine::model::OllamaLimits::new(16384, 4096).unwrap(),
        }),
        worker_concurrency: 1,
        demo_mode: true,
        metrics: Arc::new(memory_engine::observability::Metrics::default()),
    });
    let app = api::router(state);
    let (_, session) = call(
        &app,
        "POST",
        "/api/v1/sessions",
        json!({"namespace":"n1","external_id":"s1"}),
    )
    .await;
    let sid = session["id"].as_str().unwrap();
    let t = Utc::now();
    let (status, first) = call(
        &app,
        "POST",
        "/api/v1/events",
        json!({"session_id":sid,"role":"user","content":"Python","occurred_at":t}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(first["outcomes"][0]["action"], "created");
    assert!(first["trace_id"].as_str().is_some());
    let(_,repeat)=call(&app,"POST","/api/v1/events",json!({"session_id":sid,"role":"user","content":"Python again","occurred_at":t+Duration::seconds(1)})).await;
    assert_eq!(repeat["outcomes"][0]["action"], "reinforced");
    let(_,corrected)=call(&app,"POST","/api/v1/events",json!({"session_id":sid,"role":"user","content":"Rust","occurred_at":t+Duration::seconds(2)})).await;
    assert_eq!(corrected["outcomes"][0]["action"], "superseded");
    let(status,_)=call(&app,"POST","/api/v1/events",json!({"session_id":sid,"role":"user","content":"Python","occurred_at":t-Duration::seconds(1)})).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, search) = call(
        &app,
        "POST",
        "/api/v1/search",
        json!({"namespace":"n1","query":"programming language","max_tokens":100,"top_k":5}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(search["context"].as_str().unwrap().contains("Rust"));
    assert!(!search["context"].as_str().unwrap().contains("Python"));
    let trace_id = search["trace_id"].as_str().unwrap();
    let (status, trace) = call(
        &app,
        "GET",
        &format!("/api/v1/traces/{trace_id}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(trace["operation_type"], "search");
    let stages = trace["steps"].as_array().unwrap();
    assert!(stages.iter().any(|s| s["stage"] == "lexical_retrieval"));
    assert!(stages.iter().any(|s| s["stage"] == "semantic_retrieval"));
    assert!(
        stages
            .iter()
            .any(|s| s["stage"] == "reciprocal_rank_fusion")
    );
    assert!(stages.iter().any(|s| s["stage"] == "token_budget_packing"));
    let rows = store.list("n1", true).await.unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|x| x.status == "active").count(), 1);
    for version in &rows {
        let assertion = store.assertion("n1", version.id).await.unwrap();
        assert_eq!(assertion.status, version.status);
        assert_eq!(assertion.sources.len(), version.sources.len());
        assert_eq!(assertion.valid_to, version.valid_to);
        if version.value == "Rust" {
            assert!(
                assertion
                    .relations
                    .iter()
                    .any(|edge| edge.relation == "supersedes")
            );
        }
    }
    assert_eq!(
        rows.iter()
            .find(|x| x.value == "Python")
            .unwrap()
            .sources
            .len(),
        2
    );
    assert!(
        rows.iter()
            .find(|x| x.value == "Rust")
            .unwrap()
            .supersedes
            .is_some()
    );
    assert!(store.list("other", true).await.unwrap().is_empty());
    let dims:i32=sqlx::query_scalar("SELECT atttypmod FROM pg_attribute WHERE attrelid='memory_versions'::regclass AND attname='embedding'").fetch_one(&store.pool).await.unwrap();
    assert_eq!(dims, 1024);
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/events",
        json!({"session_id":sid,"role":"bogus","content":"x"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, _) = call(
        &app,
        "POST",
        "/api/v1/events",
        json!({"session_id":sid,"role":"user","content":"FAIL"}),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    let degraded = api::router(Arc::new(AppState {
        store,
        graph: None,
        extractor: Arc::new(FakeExtractor),
        embedder: Arc::new(FakeEmbedder { fail: true }),
        model: Arc::new(memory_engine::model::OllamaJsonModel {
            base: "http://127.0.0.1:1".into(),
            model: "unused".into(),
            limits: memory_engine::model::OllamaLimits::new(16384, 4096).unwrap(),
        }),
        worker_concurrency: 1,
        demo_mode: true,
        metrics: Arc::new(memory_engine::observability::Metrics::default()),
    }));
    let (status, result) = call(
        &degraded,
        "POST",
        "/api/v1/search",
        json!({"namespace":"n1","query":"programming"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(result["degraded_mode"], true);
    assert!(result["context"].as_str().unwrap().contains("Rust"));
}
