use super::*;

impl Store {
    pub async fn retain(&self, r: &RetainRequest) -> Result<(Uuid, Uuid, bool), AppError> {
        if [&r.namespace, &r.session_id, &r.external_id]
            .iter()
            .any(|x| x.trim().is_empty())
            || r.events.is_empty()
            || r.events.len() > 1000
        {
            return Err(AppError::Validation(
                "namespace, session, external ID and 1..1000 events are required".into(),
            ));
        }
        for e in &r.events {
            if !["user", "assistant", "system", "tool"].contains(&e.role.as_str())
                || e.content.trim().is_empty()
                || e.content.len() > 1_000_000
            {
                return Err(AppError::Validation(
                    "invalid evidence role or content size".into(),
                ));
            }
        }
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(r).map_err(|e| AppError::Validation(e.to_string()))?)
        );
        let mut tx = self.pool.begin().await?;
        lock_namespace(&mut tx, &r.namespace).await?;
        if let Some(row) = sqlx::query(
            "SELECT id,content_hash FROM episodes WHERE namespace=$1 AND external_id=$2",
        )
        .bind(&r.namespace)
        .bind(&r.external_id)
        .fetch_optional(&mut *tx)
        .await?
        {
            if row.get::<String, _>("content_hash") != hash {
                return Err(AppError::Conflict(
                    "idempotency key reused with different evidence".into(),
                ));
            }
            let id: Uuid = row.get("id");
            let job = sqlx::query_scalar("SELECT id FROM memory_jobs WHERE dedupe_key=$1")
                .bind(format!("extract:{id}"))
                .fetch_one(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok((id, job, false));
        }
        let session:Uuid=sqlx::query_scalar("INSERT INTO sessions(namespace,external_id) VALUES($1,$2) ON CONFLICT(namespace,external_id) DO UPDATE SET external_id=EXCLUDED.external_id RETURNING id")
            .bind(&r.namespace).bind(&r.session_id).fetch_one(&mut *tx).await?;
        let episode:Uuid=sqlx::query_scalar("INSERT INTO episodes(namespace,session_id,external_id,content_hash,metadata) VALUES($1,$2,$3,$4,$5) RETURNING id")
            .bind(&r.namespace).bind(session).bind(&r.external_id).bind(hash).bind(&r.metadata).fetch_one(&mut *tx).await?;
        for (i, e) in r.events.iter().enumerate() {
            let event:Uuid=sqlx::query_scalar("INSERT INTO raw_events(session_id,role,content,occurred_at,metadata) VALUES($1,$2,$3,$4,$5) RETURNING id")
                .bind(session).bind(&e.role).bind(&e.content).bind(e.occurred_at).bind(&e.metadata).fetch_one(&mut *tx).await?;
            let chunk: Uuid = sqlx::query_scalar(
                "INSERT INTO chunks(raw_event_id,ordinal,content) VALUES($1,0,$2) RETURNING id",
            )
            .bind(event)
            .bind(&e.content)
            .fetch_one(&mut *tx)
            .await?;
            sqlx::query("INSERT INTO episode_sources VALUES($1,$2,$3)")
                .bind(episode)
                .bind(chunk)
                .bind(i as i32)
                .execute(&mut *tx)
                .await?;
        }
        let job = enqueue(
            &mut tx,
            &r.namespace,
            "extract",
            &format!("extract:{episode}"),
            json!({"episode_id":episode}),
        )
        .await?;
        tx.commit().await?;
        Ok((episode, job, true))
    }
    pub async fn episode_evidence(
        &self,
        namespace: &str,
        episode: Uuid,
    ) -> Result<Vec<(Uuid, EvidenceEvent)>, AppError> {
        let rows = sqlx::query(
            r#"
SELECT c.id,
       e.role,
       e.content,
       e.occurred_at,
       e.metadata
FROM episode_sources es
JOIN episodes ep ON ep.id = es.episode_id
JOIN chunks c ON c.id = es.chunk_id
JOIN raw_events e ON e.id = c.raw_event_id
WHERE ep.namespace = $1
  AND ep.id = $2
ORDER BY es.ordinal
"#,
        )
        .bind(namespace)
        .bind(episode)
        .fetch_all(&self.pool)
        .await?;
        if rows.is_empty() {
            return Err(AppError::NotFound);
        }
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r.get("id"),
                    EvidenceEvent {
                        role: r.get("role"),
                        content: r.get("content"),
                        occurred_at: r.get("occurred_at"),
                        metadata: r.get("metadata"),
                    },
                )
            })
            .collect())
    }
}
