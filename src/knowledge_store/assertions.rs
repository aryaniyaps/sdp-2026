use super::*;

impl Store {
    pub async fn apply_extraction(
        &self,
        job: &Job,
        extraction: &Extraction,
        vectors: Vec<Option<Vec<f32>>>,
        model: &str,
    ) -> Result<Value, AppError> {
        let episode: Uuid = serde_json::from_value(job.payload["episode_id"].clone())
            .map_err(|e| AppError::Validation(e.to_string()))?;
        let evidence = self.episode_evidence(&job.namespace, episode).await?;
        let events: Vec<_> = evidence.iter().map(|(_, e)| e.clone()).collect();
        if vectors.len() != extraction.claims.len() {
            return Err(AppError::Validation("embedding count mismatch".into()));
        }
        for c in &extraction.claims {
            validate_claim(c, &events)?;
        }
        let mut tx = self.pool.begin().await?;
        lock_namespace(&mut tx, &job.namespace).await?;
        let owned: bool = sqlx::query_scalar(
            r#"
SELECT EXISTS
  (SELECT 1
   FROM memory_jobs j
   JOIN memory_namespace_leases l ON l.job_id = j.id
   AND l.lease_token = j.lease_token
   WHERE j.id = $1
     AND j.status = 'running'
     AND j.lease_token = $2
     AND j.lease_until > now()
     AND l.lease_until > now())
"#,
        )
        .bind(job.id)
        .bind(job.lease_token)
        .fetch_one(&mut *tx)
        .await?;
        if !owned {
            return Err(AppError::Conflict("extraction lease lost".into()));
        }
        let mut decisions = Vec::new();
        for (c, embedding) in extraction.claims.iter().zip(vectors) {
            let subject = entity(&mut tx, &job.namespace, &c.subject).await?;
            let slot = format!("{subject}::{}", normalize_component(&c.predicate));
            let value = normalize_component(&c.value);
            let at = c
                .valid_from
                .unwrap_or(events[c.source_indices[0]].occurred_at);
            let event_at = if c.cardinality == Cardinality::Event {
                Some(c.event_at.unwrap_or(at))
            } else {
                c.event_at
            };
            let same: Option<Uuid> = sqlx::query_scalar(
                r#"
SELECT id
FROM assertions
WHERE namespace = $1
  AND slot_key = $2
  AND normalized_value = $3
  AND CARDINALITY = $4
  AND status IN ('active',
                 'contested')
  AND ($4 <> 'event'
       OR event_at IS NOT DISTINCT
       FROM $5)
ORDER BY recorded_at DESC
LIMIT 1
"#,
            )
            .bind(&job.namespace)
            .bind(&slot)
            .bind(&value)
            .bind(c.cardinality.as_str())
            .bind(event_at)
            .fetch_optional(&mut *tx)
            .await?;
            let mut action = "created";
            let id = if let Some(id) = same {
                action = "reinforced";
                id
            } else {
                let previous: Vec<Uuid> = if c.cardinality == Cardinality::Single {
                    sqlx::query_scalar("SELECT id FROM assertions WHERE namespace=$1 AND slot_key=$2 AND cardinality='single' AND status IN ('active','contested') ORDER BY valid_from,id FOR UPDATE")
                        .bind(&job.namespace).bind(&slot).fetch_all(&mut *tx).await?
                } else {
                    Vec::new()
                };
                let status = if !(previous.is_empty() || c.correction && c.confidence >= 0.8) {
                    "contested"
                } else {
                    "active"
                };
                if !previous.is_empty() {
                    action = if status == "active" {
                        "superseded"
                    } else {
                        "contradicted"
                    };
                    for old in &previous {
                        // A later recording can correct a belief from before its
                        // inferred start. The old belief then has an empty validity
                        // interval, rather than a negative one; recorded_at retains
                        // when each belief entered the system.
                        sqlx::query("UPDATE assertions SET status=$2,valid_to=CASE WHEN $2='superseded' THEN GREATEST(valid_from,$3) ELSE valid_to END WHERE id=$1")
                            .bind(old).bind(if status=="active" {"superseded"} else {"contested"}).bind(at).execute(&mut *tx).await?;
                        invalidate(&mut tx, &job.namespace, *old).await?;
                        project(&mut tx, &job.namespace, *old).await?;
                    }
                }
                let id:Uuid=sqlx::query_scalar(r#"
INSERT INTO assertions(namespace, subject_id, predicate, value, normalized_value, STATEMENT, slot_key, CARDINALITY, kind, status, valid_from, event_at, confidence, model, episode_id, embedding)
VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16) RETURNING id
"#)
                    .bind(&job.namespace).bind(subject).bind(&c.predicate).bind(&c.value).bind(&value).bind(&c.statement).bind(&slot).bind(c.cardinality.as_str()).bind(&c.kind).bind(status).bind(at).bind(event_at).bind(c.confidence).bind(model).bind(episode).bind(embedding.map(Vector::from)).fetch_one(&mut *tx).await?;
                for old in previous {
                    sqlx::query("INSERT INTO assertion_edges VALUES($1,$2,$3,$4,$5,now())")
                        .bind(&job.namespace)
                        .bind(id)
                        .bind(old)
                        .bind(if status == "active" {
                            "supersedes"
                        } else {
                            "contradicts"
                        })
                        .bind(&c.explanation)
                        .execute(&mut *tx)
                        .await?;
                }
                id
            };
            for (i, q) in c.source_indices.iter().zip(&c.quotes) {
                sqlx::query(
                    "INSERT INTO assertion_sources VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
                )
                .bind(id)
                .bind(evidence[*i].0)
                .bind(q)
                .execute(&mut *tx)
                .await?;
            }
            for e in std::iter::once(&c.subject).chain(&c.entities) {
                let eid = entity(&mut tx, &job.namespace, e).await?;
                sqlx::query(
                    "INSERT INTO assertion_entities VALUES($1,$2,$3) ON CONFLICT DO NOTHING",
                )
                .bind(&job.namespace)
                .bind(id)
                .bind(eid)
                .execute(&mut *tx)
                .await?;
                for alias in &e.aliases {
                    if let Some(i) = c
                        .source_indices
                        .iter()
                        .find(|i| events[**i].content.contains(alias))
                    {
                        sqlx::query(
                            "INSERT INTO entity_aliases VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING",
                        )
                        .bind(&job.namespace)
                        .bind(eid)
                        .bind(normalize_component(alias))
                        .bind(evidence[*i].0)
                        .execute(&mut *tx)
                        .await?;
                    }
                }
            }
            for relation in &c.related {
                if !["extends", "contradicts", "causes"].contains(&relation.relation.as_str()) {
                    return Err(AppError::Validation("invalid extracted relation".into()));
                }
                // A model may link a restatement to the assertion that this claim
                // reinforced. Its evidence is already attached above; it is not
                // a relationship between two different assertions.
                if relation.assertion_id == id {
                    continue;
                }
                if relation.relation == "contradicts" {
                    let target = sqlx::query("SELECT status,valid_from FROM assertions WHERE namespace=$1 AND id=$2 FOR UPDATE").bind(&job.namespace).bind(relation.assertion_id).fetch_optional(&mut *tx).await?.ok_or(AppError::NotFound)?;
                    let status: String = target.get("status");
                    if ["active", "contested"].contains(&status.as_str()) {
                        if c.correction && c.confidence >= 0.8 {
                            sqlx::query(
                                "UPDATE assertions SET status='superseded',valid_to=GREATEST(valid_from,$2) WHERE id=$1",
                            )
                            .bind(relation.assertion_id)
                            .bind(at)
                            .execute(&mut *tx)
                            .await?;
                            sqlx::query("INSERT INTO assertion_edges(namespace,from_id,to_id,relation,explanation) VALUES($1,$2,$3,'supersedes',$4) ON CONFLICT DO NOTHING").bind(&job.namespace).bind(id).bind(relation.assertion_id).bind(&relation.explanation).execute(&mut *tx).await?;
                            action = "superseded";
                        } else {
                            sqlx::query(
                                "UPDATE assertions SET status='contested' WHERE id=ANY($1)",
                            )
                            .bind(vec![id, relation.assertion_id])
                            .execute(&mut *tx)
                            .await?;
                            invalidate(&mut tx, &job.namespace, id).await?;
                            action = "contradicted";
                        }
                        invalidate(&mut tx, &job.namespace, relation.assertion_id).await?;
                        project(&mut tx, &job.namespace, relation.assertion_id).await?;
                    }
                }
                sqlx::query("INSERT INTO assertion_edges(namespace,from_id,to_id,relation,explanation) VALUES($1,$2,$3,$4,$5) ON CONFLICT DO NOTHING").bind(&job.namespace).bind(id).bind(relation.assertion_id).bind(&relation.relation).bind(&relation.explanation).execute(&mut *tx).await?;
            }
            // A reinforcement changes provenance/mentions, so it also advances the projection fence.
            sqlx::query("UPDATE assertions SET revision=revision WHERE id=$1")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            project(&mut tx, &job.namespace, id).await?;
            decisions.push(json!({"assertion_id":id,"action":action,"explanation":c.explanation}));
        }
        sqlx::query("UPDATE raw_events SET processing_state='processed' WHERE id IN (SELECT c.raw_event_id FROM episode_sources es JOIN chunks c ON c.id=es.chunk_id WHERE es.episode_id=$1)").bind(episode).execute(&mut *tx).await?;
        enqueue(
            &mut tx,
            &job.namespace,
            "consolidate",
            &format!("consolidate:{}", job.id),
            json!({"episode_id":episode}),
        )
        .await?;
        let result = json!({"episode_id":episode,"decisions":decisions});
        sqlx::query("UPDATE memory_jobs SET status='succeeded',result=$3,error=NULL,completed_at=now(),lease_token=NULL,lease_until=NULL WHERE id=$1 AND lease_token=$2").bind(job.id).bind(job.lease_token).bind(&result).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM memory_namespace_leases WHERE namespace=$1 AND lease_token=$2")
            .bind(&job.namespace)
            .bind(job.lease_token)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result)
    }
    pub async fn assertion(&self, namespace: &str, id: Uuid) -> Result<AssertionView, AppError> {
        let r=sqlx::query("SELECT a.*,e.name subject FROM assertions a JOIN entities e ON e.id=a.subject_id WHERE a.namespace=$1 AND a.id=$2").bind(namespace).bind(id).fetch_optional(&self.pool).await?.ok_or(AppError::NotFound)?;
        let sources = sqlx::query(
            r#"
SELECT s.chunk_id,
       s.quote,
       e.role,
       se.external_id,
       e.occurred_at,
       e.metadata
FROM assertion_sources s
JOIN chunks c ON c.id = s.chunk_id
JOIN raw_events e ON e.id = c.raw_event_id
JOIN sessions se ON se.id = e.session_id
WHERE s.assertion_id = $1
ORDER BY e.occurred_at,
         c.id
"#,
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|s| EvidenceSource {
            chunk_id: s.get("chunk_id"),
            quote: s.get("quote"),
            role: s.get("role"),
            session_id: s.get("external_id"),
            occurred_at: s.get("occurred_at"),
            metadata: s.get("metadata"),
        })
        .collect();
        let relations=sqlx::query("SELECT to_id,relation,explanation FROM assertion_edges WHERE namespace=$1 AND from_id=$2 ORDER BY relation,to_id").bind(namespace).bind(id).fetch_all(&self.pool).await?.into_iter().map(|r|RelatedInput{assertion_id:r.get("to_id"),relation:r.get("relation"),explanation:r.get("explanation")}).collect();
        Ok(AssertionView {
            id,
            namespace: namespace.into(),
            subject_id: r.get("subject_id"),
            subject: r.get("subject"),
            predicate: r.get("predicate"),
            value: r.get("value"),
            statement: r.get("statement"),
            kind: r.get("kind"),
            status: r.get("status"),
            cardinality: r.get("cardinality"),
            valid_from: r.get("valid_from"),
            valid_to: r.get("valid_to"),
            event_at: r.get("event_at"),
            recorded_at: r.get("recorded_at"),
            confidence: r.get("confidence"),
            sources,
            relations,
        })
    }
    pub async fn apply_consolidation(
        &self,
        job: &Job,
        c: &Consolidation,
        vectors: Vec<Option<Vec<f32>>>,
        model: &str,
    ) -> Result<Value, AppError> {
        if vectors.len() != c.observations.len() {
            return Err(AppError::Validation("embedding count mismatch".into()));
        }
        let mut tx = self.pool.begin().await?;
        lock_namespace(&mut tx, &job.namespace).await?;
        let owned: bool = sqlx::query_scalar(
            r#"
SELECT EXISTS
  (SELECT 1
   FROM memory_jobs j
   JOIN memory_namespace_leases l ON l.job_id = j.id
   AND l.lease_token = j.lease_token
   WHERE j.id = $1
     AND j.status = 'running'
     AND j.lease_token = $2
     AND j.lease_until > now()
     AND l.lease_until > now())
"#,
        )
        .bind(job.id)
        .bind(job.lease_token)
        .fetch_one(&mut *tx)
        .await?;
        if !owned {
            return Err(AppError::Conflict("consolidation lease lost".into()));
        }
        let mut ids = Vec::new();
        for (o, v) in c.observations.iter().zip(vectors) {
            let supports: std::collections::HashSet<_> = o.supports.iter().copied().collect();
            if supports.len() < 2
                || !o.confidence.is_finite()
                || !(0.0..=1.0).contains(&o.confidence)
                || [&o.statement, &o.predicate, &o.value]
                    .iter()
                    .any(|s| s.trim().is_empty())
            {
                return Err(AppError::Validation("observation requires two distinct supports, valid confidence and nonempty fields".into()));
            }
            let rows=sqlx::query("SELECT id,valid_from,subject_id FROM assertions WHERE namespace=$1 AND id=ANY($2) AND status='active' FOR UPDATE").bind(&job.namespace).bind(&o.supports).fetch_all(&mut *tx).await?;
            if rows.len() != supports.len()
                || !rows
                    .iter()
                    .any(|r| r.get::<Uuid, _>("subject_id") == o.subject_id)
            {
                return Err(AppError::Conflict(
                    "observation evidence changed or subject not grounded".into(),
                ));
            }
            let at = rows
                .iter()
                .map(|r| r.get::<DateTime<Utc>, _>("valid_from"))
                .max()
                .unwrap();
            let slot = format!("{}::{}", o.subject_id, normalize_component(&o.predicate));
            let same: Option<Uuid> = sqlx::query_scalar(
                r#"
SELECT a.id
FROM assertions a
WHERE a.namespace = $1
  AND a.slot_key = $2
  AND a.kind = 'observation'
  AND a.status = 'active'
  AND a.statement = $3
  AND a.normalized_value = $4
  AND
    (SELECT count(*)
     FROM assertion_edges
     WHERE from_id = a.id
       AND relation = 'supports') = $5
  AND NOT EXISTS
    (SELECT 1
     FROM assertion_edges
     WHERE from_id = a.id
       AND relation = 'supports'
       AND NOT to_id = ANY($6))
ORDER BY a.id
LIMIT 1
"#,
            )
            .bind(&job.namespace)
            .bind(&slot)
            .bind(&o.statement)
            .bind(normalize_component(&o.value))
            .bind(supports.len() as i64)
            .bind(&o.supports)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(id) = same {
                for support in &supports {
                    sqlx::query("INSERT INTO assertion_sources SELECT $1,chunk_id,quote FROM assertion_sources WHERE assertion_id=$2 ON CONFLICT DO NOTHING").bind(id).bind(support).execute(&mut *tx).await?;
                    sqlx::query("INSERT INTO assertion_entities SELECT namespace,$1,entity_id FROM assertion_entities WHERE assertion_id=$2 ON CONFLICT DO NOTHING").bind(id).bind(support).execute(&mut *tx).await?;
                }
                sqlx::query("UPDATE assertions SET revision=revision WHERE id=$1")
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                project(&mut tx, &job.namespace, id).await?;
                ids.push(id);
                continue;
            }
            let previous:Vec<Uuid>=sqlx::query_scalar("SELECT id FROM assertions WHERE namespace=$1 AND slot_key=$2 AND kind='observation' AND status IN ('active','stale') FOR UPDATE").bind(&job.namespace).bind(&slot).fetch_all(&mut *tx).await?;
            for old in previous {
                // Inferred summaries retain historical evidence, but are unavailable once outdated.
                sqlx::query("UPDATE assertions SET status='stale' WHERE id=$1")
                    .bind(old)
                    .execute(&mut *tx)
                    .await?;
                invalidate(&mut tx, &job.namespace, old).await?;
                project(&mut tx, &job.namespace, old).await?;
            }
            let id:Uuid=sqlx::query_scalar(r#"
INSERT INTO assertions(namespace, subject_id, predicate, value, normalized_value, STATEMENT, slot_key, CARDINALITY, kind, status, valid_from, confidence, model, embedding)
VALUES($1,$2,$3,$4,$5,$6,$7,'multiple','observation','active',$8,$9,$10,$11) RETURNING id
"#)
                .bind(&job.namespace).bind(o.subject_id).bind(&o.predicate).bind(&o.value).bind(normalize_component(&o.value)).bind(&o.statement).bind(&slot).bind(at).bind(o.confidence).bind(model).bind(v.map(Vector::from)).fetch_one(&mut *tx).await?;
            for support in supports {
                sqlx::query("INSERT INTO assertion_edges(namespace,from_id,to_id,relation,explanation) VALUES($1,$2,$3,'supports',$4)").bind(&job.namespace).bind(id).bind(support).bind(&o.explanation).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO assertion_sources SELECT $1,chunk_id,quote FROM assertion_sources WHERE assertion_id=$2 ON CONFLICT DO NOTHING").bind(id).bind(support).execute(&mut *tx).await?;
                sqlx::query("INSERT INTO assertion_entities SELECT namespace,$1,entity_id FROM assertion_entities WHERE assertion_id=$2 ON CONFLICT DO NOTHING").bind(id).bind(support).execute(&mut *tx).await?;
            }
            // New observations only point to existing nodes: dependencies are acyclic by construction.
            project(&mut tx, &job.namespace, id).await?;
            ids.push(id);
        }
        let result = json!({"observation_ids":ids});
        sqlx::query("UPDATE memory_jobs SET status='succeeded',result=$3,error=NULL,completed_at=now(),lease_token=NULL,lease_until=NULL WHERE id=$1 AND lease_token=$2").bind(job.id).bind(job.lease_token).bind(&result).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM memory_namespace_leases WHERE namespace=$1 AND lease_token=$2")
            .bind(&job.namespace)
            .bind(job.lease_token)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(result)
    }
    /// Retracts an assertion and invalidates observations that depend on it.
    pub async fn retract(&self, namespace: &str, id: Uuid) -> Result<Vec<Uuid>, AppError> {
        let mut tx = self.pool.begin().await?;
        lock_namespace(&mut tx, namespace).await?;
        let n =
            sqlx::query("UPDATE assertions SET status='retracted' WHERE namespace=$1 AND id=$2")
                .bind(namespace)
                .bind(id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        if n == 0 {
            return Err(AppError::NotFound);
        }
        let stale = invalidate(&mut tx, namespace, id).await?;
        project(&mut tx, namespace, id).await?;
        enqueue(
            &mut tx,
            namespace,
            "consolidate",
            &format!("retraction:{id}:{}", Uuid::new_v4()),
            json!({"changed_assertion_id":id}),
        )
        .await?;
        tx.commit().await?;
        Ok(stale)
    }
}
