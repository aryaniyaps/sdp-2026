use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OperationStep {
    pub ordinal: i32,
    pub stage: String,
    pub status: String,
    pub duration_ms: i64,
    pub details: Value,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OperationTrace {
    pub id: Uuid,
    pub operation_type: String,
    pub namespace: String,
    pub session_id: Option<Uuid>,
    pub event_id: Option<Uuid>,
    pub status: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub duration_ms: i64,
    pub degraded_mode: bool,
    pub request: Value,
    pub result: Option<Value>,
    pub error: Option<String>,
    pub steps: Vec<OperationStep>,
}

pub struct TraceBuilder {
    id: Uuid,
    operation_type: String,
    namespace: String,
    session_id: Option<Uuid>,
    event_id: Option<Uuid>,
    started_at: DateTime<Utc>,
    started: std::time::Instant,
    request: Value,
    steps: Vec<OperationStep>,
}

impl TraceBuilder {
    pub fn new(
        operation_type: &str,
        namespace: &str,
        session_id: Option<Uuid>,
        request: Value,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            operation_type: operation_type.into(),
            namespace: namespace.into(),
            session_id,
            event_id: None,
            started_at: Utc::now(),
            started: std::time::Instant::now(),
            request,
            steps: Vec::new(),
        }
    }
    pub fn id(&self) -> Uuid {
        self.id
    }
    pub fn set_event(&mut self, event_id: Uuid) {
        self.event_id = Some(event_id);
    }
    pub fn step(
        &mut self,
        stage: &str,
        status: &str,
        duration: std::time::Duration,
        details: Value,
    ) {
        self.steps.push(OperationStep {
            ordinal: self.steps.len() as i32,
            stage: stage.into(),
            status: status.into(),
            duration_ms: duration.as_millis() as i64,
            details,
        });
    }
    pub fn finish(
        self,
        status: &str,
        degraded: bool,
        result: Option<Value>,
        error: Option<String>,
    ) -> OperationTrace {
        OperationTrace {
            id: self.id,
            operation_type: self.operation_type,
            namespace: self.namespace,
            session_id: self.session_id,
            event_id: self.event_id,
            status: status.into(),
            started_at: self.started_at,
            finished_at: Utc::now(),
            duration_ms: self.started.elapsed().as_millis() as i64,
            degraded_mode: degraded,
            request: self.request,
            result,
            error,
            steps: self.steps,
        }
    }
}

#[derive(Default)]
pub struct Metrics {
    pub ingestions: AtomicU64,
    pub searches: AtomicU64,
    pub failures: AtomicU64,
    pub degraded: AtomicU64,
    pub extracted_memories: AtomicU64,
    pub versions_created: AtomicU64,
    pub versions_reinforced: AtomicU64,
    pub versions_superseded: AtomicU64,
    pub ingestion_ms: AtomicU64,
    pub search_ms: AtomicU64,
}
impl Metrics {
    pub fn inc(&self, metric: &AtomicU64, value: u64) {
        metric.fetch_add(value, Ordering::Relaxed);
    }
    pub fn prometheus(&self) -> String {
        let get = |v: &AtomicU64| v.load(Ordering::Relaxed);
        format!(
            concat!(
                "# HELP memory_engine_ingestions_total Completed ingestion operations.\n# TYPE memory_engine_ingestions_total counter\nmemory_engine_ingestions_total {}\n",
                "# HELP memory_engine_searches_total Completed search operations.\n# TYPE memory_engine_searches_total counter\nmemory_engine_searches_total {}\n",
                "# HELP memory_engine_failures_total Failed operations.\n# TYPE memory_engine_failures_total counter\nmemory_engine_failures_total {}\n",
                "# HELP memory_engine_degraded_total Operations completed without embeddings.\n# TYPE memory_engine_degraded_total counter\nmemory_engine_degraded_total {}\n",
                "memory_engine_extracted_memories_total {}\nmemory_engine_versions_created_total {}\nmemory_engine_versions_reinforced_total {}\nmemory_engine_versions_superseded_total {}\n",
                "memory_engine_ingestion_duration_ms_sum {}\nmemory_engine_search_duration_ms_sum {}\n"
            ),
            get(&self.ingestions),
            get(&self.searches),
            get(&self.failures),
            get(&self.degraded),
            get(&self.extracted_memories),
            get(&self.versions_created),
            get(&self.versions_reinforced),
            get(&self.versions_superseded),
            get(&self.ingestion_ms),
            get(&self.search_ms)
        )
    }
}
