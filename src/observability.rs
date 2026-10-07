use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
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
