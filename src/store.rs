use crate::{
    AppError,
    domain::*,
    observability::{OperationStep, OperationTrace},
};
use chrono::{DateTime, Utc};
use pgvector::Vector;
use sqlx::{PgPool, Postgres, Row, Transaction};
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
        sqlx::query("INSERT INTO operations(id,operation_type,namespace,session_id,event_id,status,started_at,finished_at,duration_ms,degraded_mode,request,result,error) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)")
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
    pub async fn create_session(
        &self,
        namespace: &str,
        external_id: &str,
    ) -> Result<(Uuid, bool), AppError> {
        let existing = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM sessions WHERE namespace=$1 AND external_id=$2",
        )
        .bind(namespace)
        .bind(external_id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some(id) = existing {
            return Ok((id, false));
        }
        let id = sqlx::query_scalar(
            "INSERT INTO sessions(namespace,external_id) VALUES($1,$2) RETURNING id",
        )
        .bind(namespace)
        .bind(external_id)
        .fetch_one(&self.pool)
        .await?;
        Ok((id, true))
    }
    pub async fn begin_event(
        &self,
        session_id: Uuid,
        role: &str,
        content: &str,
        occurred_at: DateTime<Utc>,
    ) -> Result<(Uuid, Uuid), AppError> {
        let mut tx = self.pool.begin().await?;
        let event=sqlx::query_scalar("INSERT INTO raw_events(session_id,role,content,occurred_at) VALUES($1,$2,$3,$4) RETURNING id").bind(session_id).bind(role).bind(content).bind(occurred_at).fetch_one(&mut *tx).await?;
        let chunk = sqlx::query_scalar(
            "INSERT INTO chunks(raw_event_id,ordinal,content) VALUES($1,0,$2) RETURNING id",
        )
        .bind(event)
        .bind(content)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok((event, chunk))
    }
    pub async fn fail_event(&self, id: Uuid, error: &str) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE raw_events SET processing_state='failed',extraction_error=$2 WHERE id=$1",
        )
        .bind(id)
        .bind(error)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
    pub async fn complete_event(&self, id: Uuid) -> Result<(), AppError> {
        sqlx::query("UPDATE raw_events SET processing_state='processed' WHERE id=$1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
    pub async fn resolve(
        &self,
        namespace: &str,
        m: &ExtractedMemory,
        chunk: Uuid,
        at: DateTime<Utc>,
        extractor: &str,
        embedding: Option<Vec<f32>>,
    ) -> Result<ResolveOutcome, AppError> {
        self.resolve_once(namespace, m, chunk, at, extractor, embedding, None)
            .await
    }
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn resolve_once(
        &self,
        namespace: &str,
        m: &ExtractedMemory,
        chunk: Uuid,
        at: DateTime<Utc>,
        extractor: &str,
        embedding: Option<Vec<f32>>,
        receipt: Option<&str>,
    ) -> Result<ResolveOutcome, AppError> {
        let key = canonical_key(&m.subject, &m.predicate);
        let norm = normalize_component(&m.value);
        let mut tx = self.pool.begin().await?;
        if let Some(receipt) = receipt {
            sqlx::query("SELECT pg_advisory_xact_lock(hashtext($1))")
                .bind(format!("legacy:{namespace}"))
                .execute(&mut *tx)
                .await?;
            if let Some(outcome) = sqlx::query_scalar::<_,serde_json::Value>("SELECT outcome FROM legacy_resolution_receipts WHERE namespace=$1 AND receipt_key=$2").bind(namespace).bind(receipt).fetch_optional(&mut *tx).await? {
                return serde_json::from_value(outcome).map_err(|e| AppError::Validation(e.to_string()));
            }
        }

        let memory_id:Uuid=sqlx::query_scalar("INSERT INTO memories(namespace,canonical_key,subject,predicate) VALUES($1,$2,$3,$4) ON CONFLICT(namespace,canonical_key) DO UPDATE SET subject=memories.subject RETURNING id").bind(namespace).bind(&key).bind(m.subject.trim()).bind(m.predicate.trim()).fetch_one(&mut *tx).await?;
        sqlx::query("SELECT id FROM memories WHERE id=$1 FOR UPDATE")
            .bind(memory_id)
            .execute(&mut *tx)
            .await?;
        let active=sqlx::query("SELECT id,version,normalized_value,valid_from FROM memory_versions WHERE memory_id=$1 AND status='active' FOR UPDATE").bind(memory_id).fetch_optional(&mut *tx).await?;
        if let Some(row) = active {
            let id: Uuid = row.get("id");
            let version: i32 = row.get("version");
            let old: String = row.get("normalized_value");
            let valid_from: DateTime<Utc> = row.get("valid_from");
            if norm == old {
                Self::add_source(&mut tx, id, chunk).await?;
                let outcome = ResolveOutcome {
                    memory_id,
                    version_id: id,
                    version,
                    action: "reinforced".into(),
                };
                Self::legacy_receipt(&mut tx, namespace, receipt, &outcome).await?;
                tx.commit().await?;
                return Ok(outcome);
            }
            if at < valid_from {
                return Err(AppError::Conflict(
                    "out-of-order update would create an invalid validity interval".into(),
                ));
            }
            sqlx::query("UPDATE memory_versions SET status='superseded',valid_to=$2 WHERE id=$1")
                .bind(id)
                .bind(at)
                .execute(&mut *tx)
                .await?;
            let next = version + 1;
            let new_id =
                Self::insert_version(&mut tx, memory_id, next, m, &norm, at, extractor, embedding)
                    .await?;
            Self::add_source(&mut tx, new_id, chunk).await?;
            sqlx::query("INSERT INTO memory_relations(from_version_id,to_version_id,relation_type) VALUES($1,$2,'supersedes')").bind(new_id).bind(id).execute(&mut *tx).await?;
            let outcome = ResolveOutcome {
                memory_id,
                version_id: new_id,
                version: next,
                action: "superseded".into(),
            };
            Self::legacy_receipt(&mut tx, namespace, receipt, &outcome).await?;
            tx.commit().await?;
            return Ok(outcome);
        }
        let id =
            Self::insert_version(&mut tx, memory_id, 1, m, &norm, at, extractor, embedding).await?;
        Self::add_source(&mut tx, id, chunk).await?;
        let outcome = ResolveOutcome {
            memory_id,
            version_id: id,
            version: 1,
            action: "created".into(),
        };
        Self::legacy_receipt(&mut tx, namespace, receipt, &outcome).await?;
        tx.commit().await?;
        Ok(outcome)
    }
    async fn legacy_receipt(
        tx: &mut Transaction<'_, Postgres>,
        namespace: &str,
        receipt: Option<&str>,
        outcome: &ResolveOutcome,
    ) -> Result<(), AppError> {
        if let Some(receipt) = receipt {
            sqlx::query("INSERT INTO legacy_resolution_receipts(namespace,receipt_key,outcome) VALUES($1,$2,$3)").bind(namespace).bind(receipt).bind(serde_json::to_value(outcome).unwrap()).execute(&mut **tx).await?;
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    async fn insert_version(
        tx: &mut Transaction<'_, Postgres>,
        memory_id: Uuid,
        version: i32,
        m: &ExtractedMemory,
        norm: &str,
        at: DateTime<Utc>,
        extractor: &str,
        embedding: Option<Vec<f32>>,
    ) -> Result<Uuid, AppError> {
        let vector = embedding.map(Vector::from);
        Ok(sqlx::query_scalar("INSERT INTO memory_versions(memory_id,version,value,normalized_value,statement,kind,status,valid_from,extractor_version,embedding) VALUES($1,$2,$3,$4,$5,$6,'active',$7,$8,$9) RETURNING id").bind(memory_id).bind(version).bind(&m.value).bind(norm).bind(&m.statement).bind(m.kind.as_str()).bind(at).bind(extractor).bind(vector).fetch_one(&mut **tx).await?)
    }
    async fn add_source(
        tx: &mut Transaction<'_, Postgres>,
        version: Uuid,
        chunk: Uuid,
    ) -> Result<(), AppError> {
        sqlx::query("INSERT INTO memory_version_sources(memory_version_id,chunk_id) VALUES($1,$2) ON CONFLICT DO NOTHING").bind(version).bind(chunk).execute(&mut **tx).await?;
        Ok(())
    }
    pub async fn lexical(
        &self,
        namespace: &str,
        query: &str,
        session: Option<Uuid>,
        history: bool,
    ) -> Result<Vec<(Uuid, String)>, AppError> {
        let rows=sqlx::query("SELECT DISTINCT mv.id,mv.statement,ts_rank_cd(mv.search_vector,websearch_to_tsquery('english',$2)) score FROM memory_versions mv JOIN memories m ON m.id=mv.memory_id JOIN memory_version_sources mvs ON mvs.memory_version_id=mv.id JOIN chunks c ON c.id=mvs.chunk_id JOIN raw_events e ON e.id=c.raw_event_id WHERE m.namespace=$1 AND ($3 OR mv.status='active') AND ($4::uuid IS NULL OR e.session_id=$4) AND mv.search_vector @@ websearch_to_tsquery('english',$2) ORDER BY score DESC,mv.id LIMIT 20").bind(namespace).bind(query).bind(history).bind(session).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("id"), r.get("statement")))
            .collect())
    }
    pub async fn semantic(
        &self,
        namespace: &str,
        embedding: Vec<f32>,
        session: Option<Uuid>,
        history: bool,
    ) -> Result<Vec<(Uuid, String)>, AppError> {
        let vector = Vector::from(embedding);
        let rows=sqlx::query("SELECT DISTINCT mv.id,mv.statement,mv.embedding <=> $2 distance FROM memory_versions mv JOIN memories m ON m.id=mv.memory_id JOIN memory_version_sources mvs ON mvs.memory_version_id=mv.id JOIN chunks c ON c.id=mvs.chunk_id JOIN raw_events e ON e.id=c.raw_event_id WHERE m.namespace=$1 AND mv.embedding IS NOT NULL AND ($3 OR mv.status='active') AND ($4::uuid IS NULL OR e.session_id=$4) ORDER BY distance,mv.id LIMIT 20").bind(namespace).bind(vector).bind(history).bind(session).fetch_all(&self.pool).await?;
        Ok(rows
            .into_iter()
            .map(|r| (r.get("id"), r.get("statement")))
            .collect())
    }
    pub async fn list(&self, namespace: &str, history: bool) -> Result<Vec<VersionView>, AppError> {
        let ids=sqlx::query_scalar::<_,Uuid>("SELECT mv.id FROM memory_versions mv JOIN memories m ON m.id=mv.memory_id WHERE m.namespace=$1 AND ($2 OR mv.status='active') ORDER BY m.canonical_key,mv.version DESC").bind(namespace).bind(history).fetch_all(&self.pool).await?;
        let mut out = Vec::new();
        for id in ids {
            out.push(self.version(id).await?)
        }
        Ok(out)
    }
    pub async fn chain(&self, memory_id: Uuid) -> Result<Vec<VersionView>, AppError> {
        let ids = sqlx::query_scalar::<_, Uuid>(
            "SELECT id FROM memory_versions WHERE memory_id=$1 ORDER BY version DESC",
        )
        .bind(memory_id)
        .fetch_all(&self.pool)
        .await?;
        if ids.is_empty() {
            return Err(AppError::NotFound);
        }
        let mut out = Vec::new();
        for id in ids {
            out.push(self.version(id).await?)
        }
        Ok(out)
    }
    pub(crate) async fn version(&self, id: Uuid) -> Result<VersionView, AppError> {
        let r=sqlx::query("SELECT mv.*,m.canonical_key,m.subject,m.predicate,(SELECT to_version_id FROM memory_relations WHERE from_version_id=mv.id AND relation_type='supersedes') supersedes FROM memory_versions mv JOIN memories m ON m.id=mv.memory_id WHERE mv.id=$1").bind(id).fetch_one(&self.pool).await?;
        let sources=sqlx::query("SELECT c.id chunk_id,c.content quote,e.role,e.session_id,s.external_id,e.occurred_at FROM memory_version_sources mvs JOIN chunks c ON c.id=mvs.chunk_id JOIN raw_events e ON e.id=c.raw_event_id JOIN sessions s ON s.id=e.session_id WHERE mvs.memory_version_id=$1 ORDER BY e.occurred_at").bind(id).fetch_all(&self.pool).await?.into_iter().map(|x|SourceView{chunk_id:x.get("chunk_id"),quote:x.get("quote"),role:x.get("role"),session_id:x.get("session_id"),external_session_id:x.get("external_id"),occurred_at:x.get("occurred_at")}).collect();
        Ok(VersionView {
            id: r.get("id"),
            memory_id: r.get("memory_id"),
            canonical_key: r.get("canonical_key"),
            subject: r.get("subject"),
            predicate: r.get("predicate"),
            version: r.get("version"),
            value: r.get("value"),
            statement: r.get("statement"),
            kind: r.get("kind"),
            status: r.get("status"),
            valid_from: r.get("valid_from"),
            valid_to: r.get("valid_to"),
            extractor_version: r.get("extractor_version"),
            sources,
            supersedes: r.get("supersedes"),
        })
    }
    pub async fn reset(&self) -> Result<(), AppError> {
        let mut tx = self.pool.begin().await?;
        let min_revision: i64 = sqlx::query_scalar("SELECT nextval('assertion_revision_sequence')")
            .fetch_one(&mut *tx)
            .await?;
        let namespaces: Vec<String> = sqlx::query_scalar("SELECT DISTINCT namespace FROM entities")
            .fetch_all(&mut *tx)
            .await?;
        sqlx::query("TRUNCATE legacy_resolution_receipts,memory_jobs,assertion_edges,assertion_entities,assertion_sources,assertions,entity_aliases,entities,episode_sources,episodes,operation_steps,operations,memory_relations,memory_version_sources,memory_versions,memories,chunks,raw_events,sessions CASCADE").execute(&mut *tx).await?;
        for namespace in namespaces {
            crate::knowledge_store::enqueue(
                &mut tx,
                &namespace,
                "clear_graph",
                &format!("clear:{namespace}:{}", Uuid::new_v4()),
                serde_json::json!({"min_revision":min_revision}),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }
}

#[cfg(test)]
mod receipt_tests {
    use super::*;
    #[tokio::test]
    async fn retry_does_not_restore_a_superseded_legacy_value() {
        let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
            return;
        };
        let store = Store::new(
            sqlx::postgres::PgPoolOptions::new()
                .connect(&url)
                .await
                .unwrap(),
        );
        store.migrate().await.unwrap();
        let ns = format!("receipt-{}", Uuid::new_v4());
        let (session, _) = store.create_session(&ns, "session").await.unwrap();
        let at = Utc::now();
        let (_, chunk) = store
            .begin_event(session, "user", "Python, then Rust", at)
            .await
            .unwrap();
        let mut memory = ExtractedMemory {
            subject: "Ada".into(),
            predicate: "prefers".into(),
            value: "Python".into(),
            statement: "Ada prefers Python".into(),
            kind: MemoryKind::Preference,
        };
        let first = store
            .resolve_once(&ns, &memory, chunk, at, "fixture", None, Some("job-1:0"))
            .await
            .unwrap();
        memory.value = "Rust".into();
        memory.statement = "Ada prefers Rust".into();
        let second = store
            .resolve_once(
                &ns,
                &memory,
                chunk,
                at + chrono::Duration::seconds(1),
                "fixture",
                None,
                Some("job-2:0"),
            )
            .await
            .unwrap();
        memory.value = "Python".into();
        memory.statement = "Ada prefers Python".into();
        let retried = store
            .resolve_once(&ns, &memory, chunk, at, "fixture", None, Some("job-1:0"))
            .await
            .unwrap();
        assert_eq!(first.version_id, retried.version_id);
        assert_eq!(
            store.version(first.version_id).await.unwrap().status,
            "superseded"
        );
        assert_eq!(
            store.list(&ns, false).await.unwrap()[0].id,
            second.version_id
        );
        assert_eq!(store.chain(first.memory_id).await.unwrap().len(), 2);
    }
}
