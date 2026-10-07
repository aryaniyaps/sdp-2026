//! Project authoritative PostgreSQL facts into Neo4j.

use super::*;

pub(super) async fn project(state: &AppState, job: &Job) -> Result<Value, AppError> {
    let graph = state
        .graph
        .as_ref()
        .ok_or_else(|| AppError::Provider("Neo4j projection is not configured".into()))?;
    let id: Uuid = serde_json::from_value(job.payload["assertion_id"].clone())
        .map_err(|e| AppError::Validation(e.to_string()))?;
    // One MVCC snapshot ties properties, dependencies and revision together.
    let snapshot:Value=sqlx::query_scalar(r#"
SELECT (to_jsonb(a) - 'embedding' - 'search_vector') || jsonb_build_object('subject', e.name, 'sources', '[]'::JSONB, 'relations',
                                                                     (SELECT coalesce(jsonb_agg(jsonb_build_object('assertion_id', to_id, 'relation', relation, 'explanation', explanation)), '[]')
                                                                      FROM assertion_edges
                                                                      WHERE from_id = a.id),'entities',
                                                                     (SELECT coalesce(jsonb_agg(jsonb_build_object('id', en.id, 'name', en.name, 'entity_type', en.entity_type)), '[]')
                                                                      FROM assertion_entities ae
                                                                      JOIN entities en ON en.id = ae.entity_id
                                                                      WHERE ae.assertion_id = a.id))
FROM assertions a
JOIN entities e ON e.id = a.subject_id
WHERE a.namespace = $1
  AND a.id = $2
"#).bind(&job.namespace).bind(id).fetch_optional(&state.store.pool).await?.ok_or(AppError::NotFound)?;
    let assertion: AssertionView = serde_json::from_value(snapshot.clone())
        .map_err(|e| AppError::Validation(e.to_string()))?;
    let revision = snapshot["revision"].as_i64().unwrap();
    let entities: Vec<Value> = serde_json::from_value(snapshot["entities"].clone())
        .map_err(|e| AppError::Validation(e.to_string()))?;
    graph
        .project_assertion(&assertion, revision, entities)
        .await
        .map_err(|e| AppError::Provider(e.to_string()))?;
    Ok(json!({"assertion_id":id,"revision":revision}))
}

pub(super) async fn clear_graph(state: &AppState, job: &Job) -> Result<Value, AppError> {
    let graph = state
        .graph
        .as_ref()
        .ok_or_else(|| AppError::Provider("Neo4j is not configured".into()))?;
    graph
        .clear_namespace(
            &job.namespace,
            job.payload["min_revision"].as_i64().unwrap_or(0),
        )
        .await
        .map_err(|e| AppError::Provider(e.to_string()))?;
    Ok(json!({"cleared":true}))
}

pub(super) async fn rebuild(state: &AppState, job: &Job) -> Result<Value, AppError> {
    let mut tx = state.store.pool.begin().await?;
    let rows = sqlx::query("SELECT id FROM assertions WHERE namespace=$1 ORDER BY recorded_at,id")
        .bind(&job.namespace)
        .fetch_all(&mut *tx)
        .await?;
    for r in &rows {
        let id: Uuid = r.get("id");
        crate::knowledge_store::enqueue(
            &mut tx,
            &job.namespace,
            "project",
            &format!("rebuild:{}:{id}", job.id),
            json!({"assertion_id":id}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(json!({"queued":rows.len()}))
}
