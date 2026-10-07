//! PostgreSQL transactions for evidence, assertions, and background jobs.

mod assertions;
mod evidence;
mod jobs;
mod maintenance;

use crate::{AppError, domain::normalize_component, knowledge::*, store::Store};
use chrono::{DateTime, Utc};
use pgvector::Vector;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

/// What `Store::clear_namespace` did: rows deleted per table, the queued Neo4j clear job and its revision.
pub struct ClearOutcome {
    pub deleted: Value,
    pub job: Uuid,
    pub min_revision: i64,
}

async fn lock_namespace(
    tx: &mut Transaction<'_, Postgres>,
    namespace: &str,
) -> Result<(), AppError> {
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(namespace)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
pub(crate) async fn enqueue(
    tx: &mut Transaction<'_, Postgres>,
    namespace: &str,
    kind: &str,
    key: &str,
    payload: Value,
) -> Result<Uuid, AppError> {
    Ok(sqlx::query_scalar("INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload) VALUES($1,$2,$3,$4) ON CONFLICT(dedupe_key) DO UPDATE SET dedupe_key=EXCLUDED.dedupe_key RETURNING id")
        .bind(namespace).bind(kind).bind(key).bind(payload).fetch_one(&mut **tx).await?)
}
async fn project(
    tx: &mut Transaction<'_, Postgres>,
    namespace: &str,
    id: Uuid,
) -> Result<(), AppError> {
    enqueue(
        tx,
        namespace,
        "project",
        &format!("project:{id}:{}", Uuid::new_v4()),
        json!({"assertion_id":id}),
    )
    .await?;
    Ok(())
}
async fn entity(
    tx: &mut Transaction<'_, Postgres>,
    namespace: &str,
    e: &EntityInput,
) -> Result<Uuid, AppError> {
    if e.name.trim().is_empty() || e.entity_type.trim().is_empty() {
        return Err(AppError::Validation(
            "entity name and type are required".into(),
        ));
    }
    let normalized = normalize_component(&e.name);
    // Alias collisions remain separate candidates; never merge on similarity alone.
    let exact: Option<Uuid> = sqlx::query_scalar(
        "SELECT id FROM entities WHERE namespace=$1 AND entity_type=$2 AND normalized_name=$3",
    )
    .bind(namespace)
    .bind(&e.entity_type)
    .bind(&normalized)
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(id) = exact {
        return Ok(id);
    }
    let aliases: Vec<Uuid>=sqlx::query_scalar("SELECT DISTINCT e.id FROM entities e JOIN entity_aliases a ON a.entity_id=e.id WHERE e.namespace=$1 AND e.entity_type=$2 AND a.alias=$3")
        .bind(namespace).bind(&e.entity_type).bind(&normalized).fetch_all(&mut **tx).await?;
    if aliases.len() == 1 {
        return Ok(aliases[0]);
    }
    Ok(sqlx::query_scalar(
        r#"
INSERT INTO entities(namespace, entity_type, name, normalized_name)
VALUES($1,$2,$3,$4) ON CONFLICT(namespace, entity_type, normalized_name) DO
UPDATE
SET name = entities.name RETURNING id
"#,
    )
    .bind(namespace)
    .bind(&e.entity_type)
    .bind(e.name.trim())
    .bind(normalized)
    .fetch_one(&mut **tx)
    .await?)
}
async fn invalidate(
    tx: &mut Transaction<'_, Postgres>,
    namespace: &str,
    root: Uuid,
) -> Result<Vec<Uuid>, AppError> {
    let ids: Vec<Uuid> = sqlx::query_scalar(
        r#"
WITH RECURSIVE dependents(id) AS
  (SELECT from_id
   FROM assertion_edges
   WHERE namespace = $1
     AND to_id = $2
     AND relation IN ('supports',
                      'derives')
   UNION SELECT e.from_id
   FROM assertion_edges e
   JOIN dependents d ON e.to_id = d.id
   WHERE e.namespace = $1
     AND e.relation IN ('supports',
                        'derives'))
UPDATE assertions
SET status = 'stale'
WHERE namespace = $1
  AND id IN
    (SELECT id
     FROM dependents)
  AND kind = 'observation'
  AND status IN ('active',
                 'contested') RETURNING id
"#,
    )
    .bind(namespace)
    .bind(root)
    .fetch_all(&mut **tx)
    .await?;
    for id in &ids {
        project(tx, namespace, *id).await?;
    }
    Ok(ids)
}
fn job_from_row(r: sqlx::postgres::PgRow) -> Job {
    Job {
        id: r.get("id"),
        namespace: r.get("namespace"),
        kind: r.get("kind"),
        payload: r.get("payload"),
        status: r.get("status"),
        attempts: r.get("attempts"),
        lease_token: r.get("lease_token"),
        result: r.get("result"),
        error: r.get("error"),
    }
}
