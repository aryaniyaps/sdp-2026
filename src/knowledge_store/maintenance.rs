use super::*;

impl Store {
    pub async fn clear_namespace(&self, namespace: &str) -> Result<ClearOutcome, AppError> {
        if namespace.trim().is_empty() {
            return Err(AppError::Validation("namespace is required".into()));
        }
        let mut tx = self.pool.begin().await?;
        lock_namespace(&mut tx, namespace).await?;
        let running: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM memory_jobs WHERE namespace=$1 AND status='running' AND lease_until>now()",
        )
        .bind(namespace)
        .fetch_one(&mut *tx)
        .await?;
        if running > 0 {
            return Err(AppError::Conflict(format!(
                "{running} job(s) of this namespace are running; clear again when they have finished"
            )));
        }
        // Children first. Assertions take their sources, entity links and edges with them (ON DELETE CASCADE),
        // episodes take their episode_sources. Any table that still points at a chunk makes the delete fail and roll back.
        const STEPS: [(&str, &str); 14] = [
            ("jobs", "DELETE FROM memory_jobs WHERE namespace=ANY($1)"),
            (
                "legacy_receipts",
                "DELETE FROM legacy_resolution_receipts WHERE namespace=ANY($1)",
            ),
            (
                "legacy_relations",
                "DELETE FROM memory_relations WHERE from_version_id IN (SELECT mv.id FROM memory_versions mv JOIN memories m ON m.id=mv.memory_id WHERE m.namespace=ANY($1)) OR to_version_id IN (SELECT mv.id FROM memory_versions mv JOIN memories m ON m.id=mv.memory_id WHERE m.namespace=ANY($1))",
            ),
            (
                "legacy_sources",
                "DELETE FROM memory_version_sources WHERE memory_version_id IN (SELECT mv.id FROM memory_versions mv JOIN memories m ON m.id=mv.memory_id WHERE m.namespace=ANY($1))",
            ),
            (
                "legacy_versions",
                "DELETE FROM memory_versions WHERE memory_id IN (SELECT id FROM memories WHERE namespace=ANY($1))",
            ),
            (
                "legacy_memories",
                "DELETE FROM memories WHERE namespace=ANY($1)",
            ),
            (
                "assertions",
                "DELETE FROM assertions WHERE namespace=ANY($1)",
            ),
            (
                "entity_aliases",
                "DELETE FROM entity_aliases WHERE namespace=ANY($1)",
            ),
            ("entities", "DELETE FROM entities WHERE namespace=ANY($1)"),
            ("episodes", "DELETE FROM episodes WHERE namespace=ANY($1)"),
            (
                "chunks",
                "DELETE FROM chunks WHERE raw_event_id IN (SELECT re.id FROM raw_events re JOIN sessions s ON s.id=re.session_id WHERE s.namespace=ANY($1))",
            ),
            (
                "traces_unlinked",
                "UPDATE operations SET session_id=NULL, event_id=NULL WHERE namespace=ANY($1)",
            ),
            (
                "events",
                "DELETE FROM raw_events WHERE session_id IN (SELECT id FROM sessions WHERE namespace=ANY($1))",
            ),
            ("sessions", "DELETE FROM sessions WHERE namespace=ANY($1)"),
        ];
        // Older releases wrote shadow memories that reference these chunks.
        // Clear those rows too when removing a namespace from an existing database.
        let scope = vec![namespace.to_string(), format!("{namespace}:legacy")];
        let mut deleted = serde_json::Map::new();
        for (label, sql) in STEPS {
            let n = sqlx::query(sql)
                .bind(&scope)
                .execute(&mut *tx)
                .await?
                .rows_affected();
            deleted.insert(label.into(), json!(n));
        }
        let min_revision: i64 = sqlx::query_scalar("SELECT nextval('assertion_revision_sequence')")
            .fetch_one(&mut *tx)
            .await?;
        let job = enqueue(
            &mut tx,
            namespace,
            "clear_graph",
            &format!("clear:{namespace}:{}", Uuid::new_v4()),
            json!({"min_revision":min_revision}),
        )
        .await?;
        tx.commit().await?;
        Ok(ClearOutcome {
            deleted: Value::Object(deleted),
            job,
            min_revision,
        })
    }
}
