use crate::{
    AppError,
    observability::{OperationStep, OperationTrace},
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone)]
pub struct Store {
    pub pool: PgPool,
}
impl Store {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
    pub async fn migrate(&self) -> Result<(), AppError> {
        sqlx::migrate!().run(&self.pool).await?;
        Ok(())
    }
    pub async fn record_trace(&self, trace: &OperationTrace) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(r#"
INSERT INTO operations(id, operation_type, namespace, session_id, event_id, status, started_at, finished_at, duration_ms, degraded_mode, request, RESULT, error)
VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)
"#)
            .bind(trace.id).bind(&trace.operation_type).bind(&trace.namespace).bind(trace.session_id).bind(trace.event_id).bind(&trace.status).bind(trace.started_at).bind(trace.finished_at).bind(trace.duration_ms).bind(trace.degraded_mode).bind(&trace.request).bind(&trace.result).bind(&trace.error).execute(&mut *tx).await?;
        for step in &trace.steps {
            sqlx::query("INSERT INTO operation_steps(operation_id,ordinal,stage,status,duration_ms,details) VALUES($1,$2,$3,$4,$5,$6)")
                .bind(trace.id).bind(step.ordinal).bind(&step.stage).bind(&step.status).bind(step.duration_ms).bind(&step.details).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn traces(
        &self,
        namespace: &str,
        limit: i64,
    ) -> Result<Vec<OperationTrace>, AppError> {
        let ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM operations WHERE namespace=$1 ORDER BY started_at DESC LIMIT $2",
        )
        .bind(namespace)
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?;
        let mut traces = Vec::new();
        for id in ids {
            traces.push(self.trace(id).await?);
        }
        Ok(traces)
    }
    pub async fn trace(&self, id: Uuid) -> Result<OperationTrace, AppError> {
        let r = sqlx::query("SELECT * FROM operations WHERE id=$1")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(AppError::NotFound)?;
        let steps=sqlx::query("SELECT ordinal,stage,status,duration_ms,details FROM operation_steps WHERE operation_id=$1 ORDER BY ordinal").bind(id).fetch_all(&self.pool).await?.into_iter().map(|s|OperationStep{ordinal:s.get("ordinal"),stage:s.get("stage"),status:s.get("status"),duration_ms:s.get("duration_ms"),details:s.get("details")}).collect();
        Ok(OperationTrace {
            id: r.get("id"),
            operation_type: r.get("operation_type"),
            namespace: r.get("namespace"),
            session_id: r.get("session_id"),
            event_id: r.get("event_id"),
            status: r.get("status"),
            started_at: r.get("started_at"),
            finished_at: r.get("finished_at"),
            duration_ms: r.get("duration_ms"),
            degraded_mode: r.get("degraded_mode"),
            request: r.get("request"),
            result: r.get("result"),
            error: r.get("error"),
            steps,
        })
    }
}
