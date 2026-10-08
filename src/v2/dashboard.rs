//! Durable project catalog and session-scoped namespace handoff to Pi.
use super::*;
use serde::Serialize;

#[derive(Deserialize)]
pub(super) struct ProjectInput {
    id: String,
}
#[derive(Deserialize)]
pub(super) struct SessionInput {
    project: String,
    namespace: Option<String>,
}
#[derive(Deserialize)]
pub(super) struct Change {
    namespace: String,
    revision: i64,
}
#[derive(Deserialize)]
pub(super) struct Ack {
    revision: i64,
}
#[derive(Serialize, sqlx::FromRow)]
pub(super) struct Session {
    id: Uuid,
    project: String,
    namespace: String,
    revision: i64,
    applied_revision: i64,
}
fn validate_namespace(ns: &str) -> Result<(), AppError> {
    if ns.trim().is_empty() || ns.len() > 512 || ns.chars().any(char::is_control) {
        return Err(AppError::Validation(
            "namespace must contain 1–512 bytes without control characters".into(),
        ));
    }
    Ok(())
}
pub(super) async fn projects(State(s): State<Arc<AppState>>) -> Result<Json<Value>, AppError> {
    let ids =
        sqlx::query_scalar::<_, String>("SELECT id FROM dashboard_projects ORDER BY created_at,id")
            .fetch_all(&s.store.pool)
            .await?;
    Ok(Json(json!({"projects":ids})))
}
pub(super) async fn create_project(
    State(s): State<Arc<AppState>>,
    Json(input): Json<ProjectInput>,
) -> Result<Json<Value>, AppError> {
    let id = input.id;
    if id.is_empty()
        || id.len() > 64
        || !id.bytes().next().is_some_and(|c| c.is_ascii_alphanumeric())
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err(AppError::Validation("project must start with a letter or digit and use only letters, digits, - or _ (max 64)".into()));
    }
    sqlx::query("INSERT INTO dashboard_projects(id) VALUES($1) ON CONFLICT DO NOTHING")
        .bind(&id)
        .execute(&s.store.pool)
        .await?;
    Ok(Json(json!({"id":id})))
}
pub(super) async fn create_session(
    State(s): State<Arc<AppState>>,
    Json(input): Json<SessionInput>,
) -> Result<Json<Session>, AppError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM dashboard_projects WHERE id=$1)")
            .bind(&input.project)
            .fetch_one(&s.store.pool)
            .await?;
    if !exists {
        return Err(AppError::NotFound);
    }
    let ns = input
        .namespace
        .unwrap_or_else(|| format!("project:{}", input.project));
    validate_namespace(&ns)?;
    let row = sqlx::query_as(
        "INSERT INTO dashboard_sessions(id,project,namespace) VALUES($1,$2,$3) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(input.project)
    .bind(ns)
    .fetch_one(&s.store.pool)
    .await?;
    Ok(Json(row))
}
pub(super) async fn session(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
) -> Result<Json<Session>, AppError> {
    Ok(Json(
        sqlx::query_as("SELECT * FROM dashboard_sessions WHERE id=$1")
            .bind(id)
            .fetch_optional(&s.store.pool)
            .await?
            .ok_or(AppError::NotFound)?,
    ))
}
pub(super) async fn change(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(input): Json<Change>,
) -> Result<Json<Session>, AppError> {
    validate_namespace(&input.namespace)?;
    let row = sqlx::query_as("UPDATE dashboard_sessions SET namespace=$2,revision=revision+1,updated_at=now() WHERE id=$1 AND revision=$3 RETURNING *").bind(id).bind(input.namespace).bind(input.revision).fetch_optional(&s.store.pool).await?.ok_or_else(|| AppError::Conflict("session changed; refresh and retry".into()))?;
    Ok(Json(row))
}
pub(super) async fn ack(
    State(s): State<Arc<AppState>>,
    Path(id): Path<Uuid>,
    Json(input): Json<Ack>,
) -> Result<Json<Value>, AppError> {
    let result = sqlx::query("UPDATE dashboard_sessions SET applied_revision=$2,updated_at=now() WHERE id=$1 AND revision=$2").bind(id).bind(input.revision).execute(&s.store.pool).await?;
    Ok(Json(json!({"applied":result.rows_affected()==1})))
}
