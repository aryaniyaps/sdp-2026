//! Graph inspection and namespace maintenance endpoints.

use super::{Namespace, namespace_status};
use crate::{
    AppError, AppState,
    graph::{ProjectionNode, ProjectionRelationship},
};
use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};
use uuid::Uuid;

#[utoipa::path(post,path="/api/v2/graph/rebuild",responses((status=200,body=Value)),tag="Evidence memory",request_body=Namespace)]
pub(super) async fn rebuild(
    State(s): State<Arc<AppState>>,
    Json(q): Json<Namespace>,
) -> Result<Json<Value>, AppError> {
    let mut tx = s.store.pool.begin().await?;
    let id = crate::knowledge_store::enqueue(
        &mut tx,
        &q.namespace,
        "rebuild",
        &format!("rebuild:{}", Uuid::new_v4()),
        json!({}),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(json!({"job_id":id})))
}
/// Body of `POST /api/v2/graph/clear`. `confirm` must repeat `namespace` exactly, so a stray request cannot wipe a memory.
#[derive(Deserialize, utoipa::ToSchema)]
pub(super) struct ClearRequest {
    namespace: String,
    confirm: String,
}
#[utoipa::path(post,path="/api/v2/graph/clear",responses((status=200,body=Value),(status=400,description="confirm does not repeat the namespace"),(status=409,description="a job of the namespace is running")),tag="Evidence memory",request_body=ClearRequest)]
pub(super) async fn clear(
    State(s): State<Arc<AppState>>,
    Json(q): Json<ClearRequest>,
) -> Result<Json<Value>, AppError> {
    if q.namespace.trim().is_empty() || q.confirm != q.namespace {
        return Err(AppError::Validation(
            "confirm must repeat the namespace exactly".into(),
        ));
    }
    let outcome = s.store.clear_namespace(&q.namespace).await?;
    // The queued clear_graph job empties Neo4j too. Run it now as well, so the graph is empty when this
    // returns. The job is safe to run twice, and it is the retry if this call fails.
    let graph = match &s.graph {
        None => json!({"cleared":false,"reason":"Neo4j is not configured"}),
        Some(graph) => match graph
            .clear_namespace(&q.namespace, outcome.min_revision)
            .await
        {
            Ok(()) => json!({"cleared":true}),
            Err(e) => {
                json!({"cleared":false,"reason":e.to_string(),"retry":"the queued clear_graph job will try again"})
            }
        },
    };
    Ok(Json(
        json!({"namespace":q.namespace,"deleted":outcome.deleted,"clear_graph_job":outcome.job,"graph":graph}),
    ))
}
#[derive(Deserialize)]
pub(super) struct GraphQuery {
    namespace: String,
    entity_id: Option<Uuid>,
    assertion_id: Option<Uuid>,
}
#[utoipa::path(get,path="/api/v2/graph",responses((status=200,body=Value)),tag="Evidence memory",params(("namespace"=String,Query),("entity_id"=Option<Uuid>,Query),("assertion_id"=Option<Uuid>,Query)))]
pub(super) async fn graph(
    State(s): State<Arc<AppState>>,
    Query(q): Query<GraphQuery>,
) -> Result<Json<Value>, AppError> {
    let entities: Vec<Value> = sqlx::query_scalar(
        r#"
SELECT to_jsonb(e) || jsonb_build_object('aliases',
                                           (SELECT coalesce(jsonb_agg(ALIAS), '[]')
                                            FROM entity_aliases
                                            WHERE entity_id = e.id))
FROM entities e
WHERE e.namespace = $1
  AND ($2::UUID IS NULL
       OR e.id = $2)
ORDER BY e.name,
         e.id
LIMIT 100
"#,
    )
    .bind(&q.namespace)
    .bind(q.entity_id)
    .fetch_all(&s.store.pool)
    .await?;
    let assertions:Vec<Value>=sqlx::query_scalar(r#"
SELECT (to_jsonb(a) - 'embedding' - 'search_vector') || jsonb_build_object('entity_ids',
                                                                             (SELECT jsonb_agg(entity_id)
                                                                              FROM assertion_entities
                                                                              WHERE assertion_id = a.id))
FROM assertions a
WHERE a.namespace = $1
  AND ($2::UUID IS NULL
       OR EXISTS
         (SELECT 1
          FROM assertion_entities ae
          WHERE ae.assertion_id = a.id
            AND ae.entity_id = $2))
  AND ($3::UUID IS NULL
       OR a.id = $3
       OR a.id IN
         (SELECT from_id
          FROM assertion_edges
          WHERE to_id = $3)
       OR a.id IN
         (SELECT to_id
          FROM assertion_edges
          WHERE from_id = $3))
ORDER BY a.recorded_at DESC,
         a.id
LIMIT 100
"#).bind(&q.namespace).bind(q.entity_id).bind(q.assertion_id).fetch_all(&s.store.pool).await?;
    let ids: Vec<Uuid> = assertions
        .iter()
        .filter_map(|v| serde_json::from_value(v["id"].clone()).ok())
        .collect();
    let edges:Vec<Value>=sqlx::query_scalar("SELECT to_jsonb(e) FROM assertion_edges e WHERE namespace=$1 AND from_id=ANY($2) AND to_id=ANY($2)").bind(&q.namespace).bind(ids).fetch_all(&s.store.pool).await?;
    Ok(Json(
        json!({"entities":entities,"assertions":assertions,"edges":edges,"status":namespace_status(&s,&q.namespace).await?}),
    ))
}
const PROJECTION_DEFAULT_LIMIT: usize = 400;
const PROJECTION_MAX_LIMIT: usize = 2000;
const MEMORIES_DEFAULT_LIMIT: usize = 200;
const MEMORIES_MAX_LIMIT: usize = 1000;
/// Characters of chunk text returned per memory.
const MEMORY_TEXT_CHARS: i32 = 600;
/// Reads `namespace` and `limit` for the graph view endpoints. Problems are reported as JSON
/// errors rather than corrected: a missing namespace or an out of range limit is a 400.
fn graph_params(
    query: &HashMap<String, String>,
    default_limit: usize,
    max_limit: usize,
) -> Result<(String, usize), AppError> {
    let namespace = query
        .get("namespace")
        .filter(|namespace| !namespace.trim().is_empty())
        .ok_or_else(|| AppError::Validation("namespace is required".into()))?
        .clone();
    let limit = match query.get("limit") {
        None => default_limit,
        Some(raw) => raw
            .parse::<usize>()
            .ok()
            .filter(|limit| (1..=max_limit).contains(limit))
            .ok_or_else(|| {
                AppError::Validation(format!("limit must be an integer from 1 to {max_limit}"))
            })?,
    };
    Ok((namespace, limit))
}
#[derive(Serialize, utoipa::ToSchema)]
pub(super) struct ProjectionCounts {
    nodes: usize,
    relationships: usize,
}
#[derive(Serialize, utoipa::ToSchema)]
pub(super) struct ProjectionResponse {
    namespace: String,
    source: &'static str,
    nodes: Vec<ProjectionNode>,
    relationships: Vec<ProjectionRelationship>,
    counts: ProjectionCounts,
    truncated: bool,
}
/// The namespace as the Neo4j projection holds it. Unlike `/api/v2/graph`, which reads
/// PostgreSQL, this shows what the graph database serves to recall, so a projection that
/// lags behind PostgreSQL is visible. Neo4j failures are a 503, never an empty graph.
#[utoipa::path(get,path="/api/v2/graph/projection",responses((status=200,body=ProjectionResponse),(status=400),(status=503)),tag="Evidence memory",params(("namespace"=String,Query),("limit"=Option<usize>,Query,description="Maximum nodes, 1 to 2000, default 400. A value outside the range is rejected with 400, it is never clamped")))]
pub(super) async fn graph_projection(
    State(s): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<ProjectionResponse>, AppError> {
    let (namespace, limit) = graph_params(&query, PROJECTION_DEFAULT_LIMIT, PROJECTION_MAX_LIMIT)?;
    let graph = s
        .graph
        .as_ref()
        .ok_or_else(|| AppError::Unavailable("Neo4j projection is not configured".into()))?;
    let projection = graph
        .projection(&namespace, limit)
        .await
        .map_err(|e| AppError::Unavailable(format!("Neo4j: {e}")))?;
    Ok(Json(ProjectionResponse {
        namespace,
        source: "neo4j",
        counts: ProjectionCounts {
            nodes: projection.nodes.len(),
            relationships: projection.relationships.len(),
        },
        nodes: projection.nodes,
        relationships: projection.relationships,
        truncated: projection.truncated,
    }))
}
#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub(super) struct MemorySupport {
    assertion_id: Uuid,
    quote: String,
}
/// How far extraction of a memory has got, read from the extract job of its episode.
#[derive(Serialize, utoipa::ToSchema)]
pub(super) struct MemoryExtraction {
    /// "succeeded", "pending" (queued), "running", "failed" (all attempts used), "blocked"
    /// (queued behind a failed extract job of the namespace, which the worker never skips) or
    /// "none" (no extract job, as for events written through the V1 API).
    status: &'static str,
    /// Last error of a failed or retrying job. For "blocked" it is the error of the failed job
    /// that is in the way.
    error: Option<String>,
    attempts: Option<i32>,
    max_attempts: Option<i32>,
}
#[derive(Serialize, utoipa::ToSchema)]
pub(super) struct Memory {
    /// The source chunk id.
    id: Uuid,
    session_id: Option<Uuid>,
    role: String,
    occurred_at: DateTime<Utc>,
    /// The chunk content cut to 600 characters.
    text: String,
    text_truncated: bool,
    /// The state PostgreSQL keeps on the raw event. Only a successful extraction changes it,
    /// so a failed extraction still reads "pending" here. Use `extraction` to tell them apart.
    processing_state: Option<String>,
    extraction: MemoryExtraction,
    /// Assertions of the namespace that cite this chunk, one entry per quote. Empty until
    /// extraction has run.
    supports: Vec<MemorySupport>,
}
/// Open background jobs of the namespace. `extract_pending` does not include the jobs that
/// are `extract_blocked`.
#[derive(Serialize, Default, utoipa::ToSchema)]
pub(super) struct MemoryJobCounts {
    extract_pending: usize,
    extract_running: usize,
    extract_failed: usize,
    extract_blocked: usize,
    project_pending: usize,
    project_running: usize,
    project_failed: usize,
}
#[derive(Serialize, utoipa::ToSchema)]
pub(super) struct MemoriesCounts {
    memories: usize,
    supported: usize,
    /// Every assertion PostgreSQL holds for the namespace, so a viewer can tell when the
    /// Neo4j projection is behind. Not limited by `limit`.
    assertions: usize,
    /// The same assertions by status, to spot a projection that lags with an equal total.
    assertions_by_status: BTreeMap<String, usize>,
    jobs: MemoryJobCounts,
}
#[derive(Serialize, utoipa::ToSchema)]
pub(super) struct MemoriesResponse {
    namespace: String,
    source: &'static str,
    memories: Vec<Memory>,
    counts: MemoriesCounts,
    truncated: bool,
}
/// Chunks of a namespace are its cited chunks plus the user and assistant chunks of its
/// episodes. A quarter of `limit` is reserved for chunks that no assertion cites, unprocessed
/// ones first and then the newest, so a memory that was just retained is listed however many
/// chunks are cited. The rest goes to cited chunks, newest first, then to any chunk left.
/// One more row than `limit` is read to learn whether the list was cut. The page of chunks is
/// chosen from ids and times only, content and citations are read for the chosen rows.
const MEMORIES_SQL: &str = "\
WITH cited AS MATERIALIZED (SELECT DISTINCT s.chunk_id FROM assertion_sources s JOIN assertions a ON a.id=s.assertion_id WHERE a.namespace=$1), \
pool AS MATERIALIZED (SELECT c.id,e.occurred_at,e.processing_state,ci.chunk_id IS NOT NULL AS cited \
 FROM (SELECT chunk_id FROM cited UNION SELECT es.chunk_id FROM episode_sources es JOIN episodes ep ON ep.id=es.episode_id WHERE ep.namespace=$1) w \
 JOIN chunks c ON c.id=w.chunk_id JOIN raw_events e ON e.id=c.raw_event_id LEFT JOIN cited ci ON ci.chunk_id=c.id \
 WHERE ci.chunk_id IS NOT NULL OR e.role IN ('user','assistant')), \
reserved AS (SELECT id FROM pool WHERE NOT cited ORDER BY processing_state<>'processed' DESC,occurred_at DESC,id LIMIT $4), \
picked AS (SELECT p.id,(r.id IS NOT NULL) AS held,p.cited,p.occurred_at FROM pool p LEFT JOIN reserved r ON r.id=p.id \
 ORDER BY (r.id IS NOT NULL) DESC,p.cited DESC,p.occurred_at DESC,p.id LIMIT $2), \
ranked AS (SELECT id,row_number() OVER (ORDER BY held DESC,cited DESC,occurred_at DESC,id) AS rank FROM picked) \
SELECT r.id,r.rank,e.session_id,e.role,e.occurred_at,left(c.content,$3) AS text,char_length(c.content)>$3 AS text_truncated,e.processing_state,\
x.status AS job_status,x.error AS job_error,x.attempts AS job_attempts,x.max_attempts AS job_max_attempts,x.blocked AS job_blocked,x.blocked_error AS job_blocked_error,\
(SELECT coalesce(jsonb_agg(jsonb_build_object('assertion_id',s.assertion_id,'quote',s.quote) ORDER BY s.assertion_id,s.quote),'[]') FROM assertion_sources s JOIN assertions a ON a.id=s.assertion_id WHERE s.chunk_id=r.id AND a.namespace=$1) AS supports \
FROM ranked r JOIN chunks c ON c.id=r.id JOIN raw_events e ON e.id=c.raw_event_id \
LEFT JOIN LATERAL (SELECT j.status,left(j.error,500) AS error,j.attempts,j.max_attempts,\
 (j.status='pending' AND EXISTS(SELECT 1 FROM memory_jobs f WHERE f.namespace=j.namespace AND f.kind='extract' AND f.status='failed' AND (f.created_at,f.id)<(j.created_at,j.id))) AS blocked,\
 (SELECT left(f.error,500) FROM memory_jobs f WHERE j.status='pending' AND f.namespace=j.namespace AND f.kind='extract' AND f.status='failed' AND (f.created_at,f.id)<(j.created_at,j.id) ORDER BY f.created_at,f.id LIMIT 1) AS blocked_error \
 FROM episode_sources es JOIN memory_jobs j ON j.dedupe_key='extract:'||es.episode_id::text \
 WHERE es.chunk_id=r.id AND j.kind='extract' AND j.namespace=$1 \
 ORDER BY CASE j.status WHEN 'failed' THEN 0 WHEN 'running' THEN 1 WHEN 'pending' THEN 2 ELSE 3 END LIMIT 1) x ON true \
ORDER BY e.occurred_at DESC,r.id";
fn memory_extraction(row: &sqlx::postgres::PgRow) -> Result<MemoryExtraction, AppError> {
    let status: Option<String> = row.try_get("job_status")?;
    let blocked: Option<bool> = row.try_get("job_blocked")?;
    let (status, error) = match status.as_deref() {
        None => ("none", None),
        Some("succeeded") => ("succeeded", None),
        Some("running") => ("running", row.try_get("job_error")?),
        Some("failed") => ("failed", row.try_get("job_error")?),
        Some("pending") if blocked == Some(true) => ("blocked", row.try_get("job_blocked_error")?),
        Some("pending") => ("pending", row.try_get("job_error")?),
        Some(other) => {
            return Err(AppError::Database(sqlx::Error::Decode(
                format!("unknown extract job status {other:?}").into(),
            )));
        }
    };
    Ok(MemoryExtraction {
        status,
        error,
        attempts: row.try_get("job_attempts")?,
        max_attempts: row.try_get("job_max_attempts")?,
    })
}
/// The source text a namespace holds, read from PostgreSQL. A chunk belongs to a namespace
/// through its episode. Every chunk cited by an assertion of the namespace is a candidate,
/// plus the user and assistant chunks of its episodes. When `limit` cuts the result, a quarter
/// of it is kept for the chunks no assertion cites yet (unprocessed first, then newest), so
/// evidence that extraction has not reached still appears with an empty `supports` list, and
/// `truncated` is true. Each memory says how its extraction went, and `counts` carries what
/// a viewer needs to tell a lagging projection or a stuck extraction from a quiet namespace.
#[utoipa::path(get,path="/api/v2/graph/memories",responses((status=200,body=MemoriesResponse),(status=400),(status=500)),tag="Evidence memory",params(("namespace"=String,Query),("limit"=Option<usize>,Query,description="Maximum memories, 1 to 1000, default 200. A value outside the range is rejected with 400, it is never clamped")))]
pub(super) async fn graph_memories(
    State(s): State<Arc<AppState>>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Json<MemoriesResponse>, AppError> {
    let (namespace, limit) = graph_params(&query, MEMORIES_DEFAULT_LIMIT, MEMORIES_MAX_LIMIT)?;
    let reserved = (limit / 4).max(1).min(limit.saturating_sub(1));
    let rows = sqlx::query(MEMORIES_SQL)
        .bind(&namespace)
        .bind(limit as i64 + 1)
        .bind(MEMORY_TEXT_CHARS)
        .bind(reserved as i64)
        .fetch_all(&s.store.pool)
        .await?;
    let mut truncated = false;
    let mut memories = Vec::with_capacity(rows.len());
    for row in &rows {
        if row.try_get::<i64, _>("rank")? > limit as i64 {
            truncated = true;
            continue;
        }
        let supports: sqlx::types::Json<Vec<MemorySupport>> = row.try_get("supports")?;
        memories.push(Memory {
            id: row.try_get("id")?,
            session_id: row.try_get("session_id")?,
            role: row.try_get("role")?,
            occurred_at: row.try_get("occurred_at")?,
            text: row.try_get("text")?,
            text_truncated: row.try_get("text_truncated")?,
            processing_state: row.try_get("processing_state")?,
            extraction: memory_extraction(row)?,
            supports: supports.0,
        });
    }
    let mut assertions_by_status = BTreeMap::new();
    for row in sqlx::query(
        "SELECT status,count(*) AS n FROM assertions WHERE namespace=$1 GROUP BY status",
    )
    .bind(&namespace)
    .fetch_all(&s.store.pool)
    .await?
    {
        assertions_by_status.insert(
            row.try_get::<String, _>("status")?,
            row.try_get::<i64, _>("n")? as usize,
        );
    }
    let mut jobs = MemoryJobCounts::default();
    for row in sqlx::query(
        "SELECT j.kind,j.status,count(*) AS n,\
         count(*) FILTER (WHERE j.kind='extract' AND j.status='pending' AND EXISTS(SELECT 1 FROM memory_jobs f WHERE f.namespace=j.namespace AND f.kind='extract' AND f.status='failed' AND (f.created_at,f.id)<(j.created_at,j.id))) AS blocked \
         FROM memory_jobs j WHERE j.namespace=$1 AND j.kind IN ('extract','project') AND j.status IN ('pending','running','failed') GROUP BY j.kind,j.status",
    )
    .bind(&namespace)
    .fetch_all(&s.store.pool)
    .await?
    {
        let kind: String = row.try_get("kind")?;
        let status: String = row.try_get("status")?;
        let n = row.try_get::<i64, _>("n")? as usize;
        let blocked = row.try_get::<i64, _>("blocked")? as usize;
        match (kind.as_str(), status.as_str()) {
            ("extract", "pending") => {
                jobs.extract_pending = n - blocked;
                jobs.extract_blocked = blocked;
            }
            ("extract", "running") => jobs.extract_running = n,
            ("extract", "failed") => jobs.extract_failed = n,
            ("project", "pending") => jobs.project_pending = n,
            ("project", "running") => jobs.project_running = n,
            ("project", "failed") => jobs.project_failed = n,
            other => {
                return Err(AppError::Database(sqlx::Error::Decode(
                    format!("unexpected job count row {other:?}").into(),
                )));
            }
        }
    }
    let supported = memories.iter().filter(|m| !m.supports.is_empty()).count();
    Ok(Json(MemoriesResponse {
        namespace,
        source: "postgres",
        counts: MemoriesCounts {
            memories: memories.len(),
            supported,
            assertions: assertions_by_status.values().sum(),
            assertions_by_status,
            jobs,
        },
        truncated,
        memories,
    }))
}
