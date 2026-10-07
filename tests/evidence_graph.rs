use async_trait::async_trait;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chrono::{Duration, Utc};
use http_body_util::BodyExt;
use memory_engine::{
    AppError, AppState, api,
    graph::GraphStore,
    knowledge::*,
    model::JsonModel,
    providers::Embedder,
    store::Store,
    v2::{RecallRequest, RecallResponse, recall_engine},
};
use pgvector::Vector;
use serde_json::{Value, json};
use sqlx::{Row, postgres::PgPoolOptions};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};
use tower::ServiceExt;
use uuid::Uuid;
/// The claim test takes any ready extract job in the shared database, so no test may create one
/// while it runs. Retaining evidence holds this for reading and the claim test holds it for
/// writing. A guard is held only around the call that creates the job, never across another
/// acquisition, so a waiting writer cannot deadlock a reader.
static READY_EXTRACT_JOBS: tokio::sync::RwLock<()> = tokio::sync::RwLock::const_new(());
struct NoModel;
struct ForeignCitation;
#[async_trait]
impl JsonModel for ForeignCitation {
    async fn generate(&self, _: &str) -> Result<Value, AppError> {
        Ok(json!({"answer":"Rust","citations":[Uuid::nil()],"insufficient_evidence":false}))
    }
    fn identity(&self) -> String {
        "foreign-citation-fixture".into()
    }
}
#[async_trait]
impl JsonModel for NoModel {
    async fn generate(&self, _: &str) -> Result<Value, AppError> {
        Err(AppError::Provider("no inference in fixture tests".into()))
    }
    fn identity(&self) -> String {
        "test".into()
    }
}
struct NoProviders;
#[async_trait]
impl Embedder for NoProviders {
    async fn embed(&self, _: &str) -> Result<Vec<f32>, AppError> {
        Err(AppError::Provider("offline".into()))
    }
    async fn ready(&self) -> bool {
        false
    }
    fn version(&self) -> &str {
        "test"
    }
}
fn claim(value: &str, cardinality: Cardinality, correction: bool) -> ClaimInput {
    ClaimInput {
        subject: EntityInput {
            name: "Ada".into(),
            entity_type: "person".into(),
            aliases: vec![],
        },
        predicate: "uses language".into(),
        value: value.into(),
        statement: format!("Ada uses {value}"),
        cardinality,
        kind: "fact".into(),
        confidence: 0.95,
        valid_from: None,
        event_at: None,
        source_indices: vec![0],
        quotes: vec![format!("Ada uses {value}")],
        entities: vec![],
        correction,
        explanation: "explicit source".into(),
        related: vec![],
    }
}
async fn extract(
    store: &Store,
    namespace: &str,
    text: &str,
    at: chrono::DateTime<Utc>,
    claims: Vec<ClaimInput>,
) -> (Uuid, Value) {
    let request = RetainRequest {
        namespace: namespace.into(),
        session_id: "session".into(),
        external_id: Uuid::new_v4().to_string(),
        metadata: json!({}),
        events: vec![EvidenceEvent {
            role: "user".into(),
            content: text.into(),
            occurred_at: at,
            metadata: json!({}),
        }],
    };
    let ready = READY_EXTRACT_JOBS.read().await;
    let (episode, job, _) = store.retain(&request).await.unwrap();
    // Claim only this fixture's job; integration tests may share a database without resets.
    sqlx::query("UPDATE memory_jobs SET status='running',attempts=1,lease_token=gen_random_uuid(),lease_until=now()+interval '15 minutes' WHERE id=$1").bind(job).execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO memory_namespace_leases SELECT namespace,id,lease_token,lease_until FROM memory_jobs WHERE id=$1 ON CONFLICT(namespace) DO UPDATE SET job_id=excluded.job_id,lease_token=excluded.lease_token,lease_until=excluded.lease_until").bind(job).execute(&store.pool).await.unwrap();
    drop(ready);
    let job = store.job(namespace, job).await.unwrap();
    let count = claims.len();
    let result = store
        .apply_extraction(&job, &Extraction { claims }, vec![None; count], "test")
        .await
        .unwrap();
    (episode, result)
}
async fn consolidate(
    store: &Store,
    namespace: &str,
    subject: Uuid,
    topic: &str,
    supports: Vec<Uuid>,
) -> Uuid {
    let job_id:Uuid=sqlx::query_scalar("INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload,status,attempts,lease_token,lease_until) VALUES($1,'consolidate',$2,'{}','running',1,gen_random_uuid(),now()+interval '15 minutes') RETURNING id").bind(namespace).bind(Uuid::new_v4().to_string()).fetch_one(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO memory_namespace_leases SELECT namespace,id,lease_token,lease_until FROM memory_jobs WHERE id=$1 ON CONFLICT(namespace) DO UPDATE SET job_id=excluded.job_id,lease_token=excluded.lease_token,lease_until=excluded.lease_until").bind(job_id).execute(&store.pool).await.unwrap();
    let job = store.job(namespace, job_id).await.unwrap();
    let result = store
        .apply_consolidation(
            &job,
            &Consolidation {
                observations: vec![ObservationInput {
                    subject_id: subject,
                    statement: format!("Ada {topic}"),
                    predicate: topic.into(),
                    value: topic.into(),
                    confidence: 0.8,
                    supports,
                    explanation: "combines two sources".into(),
                }],
            },
            vec![None],
            "test",
        )
        .await
        .unwrap();
    serde_json::from_value(result["observation_ids"][0].clone()).unwrap()
}
#[tokio::test]
async fn evidence_lifecycle_and_projection() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; integration skipped");
        return;
    };
    let store = Store::new(
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .unwrap(),
    );
    store.migrate().await.unwrap();
    let ns = format!("evidence-{}", Uuid::new_v4());
    let at = Utc::now();
    let req = RetainRequest {
        namespace: ns.clone(),
        session_id: "idempotency".into(),
        external_id: "same".into(),
        metadata: json!({}),
        events: vec![EvidenceEvent {
            role: "tool".into(),
            content: "tests passed".into(),
            occurred_at: at,
            metadata: json!({"exit_code":0}),
        }],
    };
    let ready = READY_EXTRACT_JOBS.read().await;
    let first = store.retain(&req).await.unwrap();
    drop(ready);
    assert!(first.2);
    assert_eq!(store.retain(&req).await.unwrap(), (first.0, first.1, false));
    let mut changed = req.clone();
    changed.events[0].content = "failed".into();
    assert!(matches!(
        store.retain(&changed).await,
        Err(AppError::Conflict(_))
    ));
    let (_, r) = extract(
        &store,
        &ns,
        "Ada uses Python",
        at,
        vec![claim("Python", Cardinality::Single, false)],
    )
    .await;
    let python: Uuid = serde_json::from_value(r["decisions"][0]["assertion_id"].clone()).unwrap();
    let mut restatement = claim("Python", Cardinality::Single, false);
    restatement.related.push(RelatedInput {
        assertion_id: python,
        relation: "extends".into(),
        explanation: "same assertion with another source".into(),
    });
    let (_, reinforced) = extract(&store, &ns, "Ada uses Python", at, vec![restatement]).await;
    assert_eq!(reinforced["decisions"][0]["action"], "reinforced");
    assert_eq!(store.assertion(&ns, python).await.unwrap().sources.len(), 2);
    let mut additive = claim("SQL", Cardinality::Multiple, false);
    additive.predicate = "knows language".into();
    let (_, r) = extract(
        &store,
        &ns,
        "Ada uses SQL",
        at + Duration::seconds(1),
        vec![additive],
    )
    .await;
    let sql: Uuid = serde_json::from_value(r["decisions"][0]["assertion_id"].clone()).unwrap();
    let subject = store.assertion(&ns, python).await.unwrap().subject_id;
    let observation = consolidate(
        &store,
        &ns,
        subject,
        "has language experience",
        vec![python, sql],
    )
    .await;
    let descendant = consolidate(
        &store,
        &ns,
        subject,
        "can combine languages",
        vec![observation, sql],
    )
    .await;
    let repeated = consolidate(
        &store,
        &ns,
        subject,
        "has language experience",
        vec![python, sql],
    )
    .await;
    assert_eq!(repeated, observation);
    assert_eq!(
        store.assertion(&ns, descendant).await.unwrap().status,
        "active"
    );
    let (_, r) = extract(
        &store,
        &ns,
        "Ada uses Rust now instead of Python",
        at + Duration::seconds(2),
        vec![claim("Rust", Cardinality::Single, true)],
    )
    .await;
    assert_eq!(r["decisions"][0]["action"], "superseded");
    let rust: Uuid = serde_json::from_value(r["decisions"][0]["assertion_id"].clone()).unwrap();
    assert_eq!(
        store.assertion(&ns, python).await.unwrap().status,
        "superseded"
    );
    for id in [observation, descendant] {
        assert_eq!(store.assertion(&ns, id).await.unwrap().status, "stale");
    }
    assert_eq!(store.assertion(&ns, sql).await.unwrap().status, "active");
    assert!(store.assertion("another-namespace", rust).await.is_err());
    let mut s = AppState {
        store: store.clone(),
        graph: None,
        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
    };
    let mut query: RecallRequest = serde_json::from_value(
        json!({"namespace":ns,"query":"Python OR Rust","graph":false,"temporal":false}),
    )
    .unwrap();
    let fallback = recall_engine(
        &s,
        serde_json::from_value(
            json!({"namespace":ns,"query":"tests passed","graph":false,"temporal":false}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(fallback.results[0].kind, "source_chunk");
    assert_eq!(fallback.results[0].sources[0].metadata["exit_code"], 0);
    assert!(fallback.results[0].ranks.contains_key("raw_fallback"));
    let current = recall_engine(&s, query.clone()).await.unwrap();
    assert!(
        !current
            .ranking
            .iter()
            .any(|h| h.id == python || h.id == observation || h.id == descendant)
    );
    query.as_of = Some(at + Duration::seconds(1));
    let historical = recall_engine(&s, query).await.unwrap();
    assert!(historical.ranking.iter().any(|h| h.id == python));
    assert!(!historical.ranking.iter().any(|h| h.id == rust));
    let abstention = memory_engine::v2::reflect_engine(
        &s,
        serde_json::from_value(
            json!({"namespace":ns,"query":"zzunseenqq","graph":false,"temporal":false}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(abstention["insufficient_evidence"], true);
    assert_eq!(abstention["citations"], json!([]));
    s.model = Arc::new(ForeignCitation);
    assert!(matches!(
        memory_engine::v2::reflect_engine(
            &s,
            serde_json::from_value(
                json!({"namespace":ns,"query":"Rust","graph":false,"temporal":false})
            )
            .unwrap()
        )
        .await,
        Err(AppError::Provider(_))
    ));
    let stale = store.retract(&ns, sql).await.unwrap();
    assert!(stale.is_empty());
    assert_eq!(store.assertion(&ns, sql).await.unwrap().status, "retracted");
    let cross_ns = format!("cross-predicate-{ns}");
    let (_, first) = extract(
        &store,
        &cross_ns,
        "Ada uses Python",
        at,
        vec![claim("Python", Cardinality::Single, false)],
    )
    .await;
    let original: Uuid =
        serde_json::from_value(first["decisions"][0]["assertion_id"].clone()).unwrap();
    let mut corrected = claim("Rust", Cardinality::Single, true);
    corrected.predicate = "preferred technology".into();
    corrected.related.push(RelatedInput {
        assertion_id: original,
        relation: "contradicts".into(),
        explanation: "explicit replacement expressed under another predicate".into(),
    });
    let (_, replacement) = extract(
        &store,
        &cross_ns,
        "Ada uses Rust instead of Python",
        at + Duration::seconds(3),
        vec![corrected],
    )
    .await;
    assert_eq!(replacement["decisions"][0]["action"], "superseded");
    assert_eq!(
        store.assertion(&cross_ns, original).await.unwrap().status,
        "superseded"
    );
    let wrong = EntityInput {
        name: "".into(),
        entity_type: "person".into(),
        aliases: vec![],
    };
    let mut invalid = claim("bad", Cardinality::Single, false);
    invalid.subject = wrong;
    assert!(validate_claim(&invalid, &req.events).is_err());
    if let Ok(uri) = std::env::var("TEST_NEO4J_URL") {
        let graph =
            memory_engine::graph::GraphStore::new(uri, "neo4j".into(), "password".into(), None);
        graph.ensure_constraints().await.unwrap();
        let a = store.assertion(&ns, rust).await.unwrap();
        let entities = vec![json!({"id":subject,"name":"Ada","entity_type":"person"})];
        graph
            .project_assertion(&a, 200, entities.clone())
            .await
            .unwrap();
        let mut old = a.clone();
        old.status = "superseded".into();
        graph.project_assertion(&old, 100, entities).await.unwrap();
        let result = graph
            .execute_cypher(
                "MATCH (a:Assertion {namespace:$ns,id:$id}) RETURN a.status,a.revision",
                json!({"ns":ns,"id":rust}),
            )
            .await
            .unwrap();
        assert_eq!(
            result["results"][0]["data"][0]["row"],
            json!(["active", 200])
        );
        graph.clear_namespace(&ns, 201).await.unwrap();
        graph.project_assertion(&a, 200, vec![]).await.unwrap();
        let result = graph
            .execute_cypher(
                "MATCH (a:Assertion {namespace:$ns}) RETURN count(a)",
                json!({"ns":ns}),
            )
            .await
            .unwrap();
        assert_eq!(result["results"][0]["data"][0]["row"], json!([0]));
        graph.project_assertion(&a, 202, vec![]).await.unwrap();
        let result = graph
            .execute_cypher(
                "MATCH (a:Assertion {namespace:$ns,id:$id}) RETURN a.revision",
                json!({"ns":ns,"id":rust}),
            )
            .await
            .unwrap();
        assert_eq!(result["results"][0]["data"][0]["row"], json!([202]));
        graph
            .project_assertion(
                &store.assertion(&ns, python).await.unwrap(),
                300,
                vec![json!({"id":subject,"name":"Ada","entity_type":"person"})],
            )
            .await
            .unwrap();
        graph
            .project_assertion(
                &store.assertion(&ns, sql).await.unwrap(),
                301,
                vec![json!({"id":subject,"name":"Ada","entity_type":"person"})],
            )
            .await
            .unwrap();
        let expanded = graph.neighbors(&ns, &[rust]).await.unwrap();
        assert!(
            expanded["results"][0]["data"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["row"][0] == json!(sql)
                    && row["row"][1].as_array().unwrap().len() == 4)
        );
        let relation: Value = sqlx::query("SELECT to_id FROM assertion_edges WHERE from_id=$1")
            .bind(rust)
            .fetch_one(&store.pool)
            .await
            .map(|r| json!(r.get::<Uuid, _>("to_id")))
            .unwrap();
        assert_eq!(relation, json!(python));
    }
}

#[tokio::test]
async fn expired_leases_recover_and_old_tokens_cannot_complete_work() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    let store = Store::new(PgPoolOptions::new().connect(&url).await.unwrap());
    store.migrate().await.unwrap();
    let ns = format!("lease-{}", Uuid::new_v4());
    let recover:Uuid=sqlx::query_scalar("INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload,status,attempts,lease_token,lease_until) VALUES($1,'rebuild',$2,'{}','running',1,gen_random_uuid(),now()-interval '1 minute') RETURNING id").bind(&ns).bind(Uuid::new_v4().to_string()).fetch_one(&store.pool).await.unwrap();
    let exhausted:Uuid=sqlx::query_scalar("INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload,status,attempts,lease_token,lease_until) VALUES($1,'rebuild',$2,'{}','running',5,gen_random_uuid(),now()-interval '1 minute') RETURNING id").bind(&ns).bind(Uuid::new_v4().to_string()).fetch_one(&store.pool).await.unwrap();
    let old = store.job(&ns, recover).await.unwrap();
    let reclaimed = store.claim_job(&["rebuild"]).await.unwrap().unwrap();
    assert_eq!(reclaimed.id, recover);
    assert_ne!(reclaimed.lease_token, old.lease_token);
    store
        .finish_job(&old, Ok(json!({"wrong":"stale worker"})))
        .await
        .unwrap();
    assert_eq!(store.job(&ns, recover).await.unwrap().status, "running");
    store
        .finish_job(&reclaimed, Ok(json!({"recovered":true})))
        .await
        .unwrap();
    assert_eq!(store.job(&ns, recover).await.unwrap().status, "succeeded");
    assert_eq!(store.job(&ns, exhausted).await.unwrap().status, "failed");
    store.retry_job(&ns, exhausted).await.unwrap();
    let restarted = store.claim_job(&["rebuild"]).await.unwrap().unwrap();
    assert_eq!(restarted.id, exhausted);
    assert_eq!(restarted.attempts, 1);
    store
        .finish_job(&restarted, Ok(json!({"recovered":true})))
        .await
        .unwrap();
}

#[tokio::test]
async fn parallel_claims_serialize_a_bank_and_preserve_history_order() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    let store = Store::new(
        PgPoolOptions::new()
            .max_connections(8)
            .connect(&url)
            .await
            .unwrap(),
    );
    store.migrate().await.unwrap();
    let _exclusive = READY_EXTRACT_JOBS.write().await;
    // Existing lifecycle fixtures are not live workers; defer them for this claim test.
    sqlx::query("UPDATE memory_jobs SET available_at=now()+interval '1 day' WHERE status='pending' AND kind IN ('extract','consolidate')").execute(&store.pool).await.unwrap();
    let ns = format!("parallel-{}", Uuid::new_v4());
    let other = format!("parallel-other-{}", Uuid::new_v4());
    let mut ids = Vec::new();
    for bank in [&ns, &ns, &other] {
        let id:Uuid=sqlx::query_scalar("INSERT INTO memory_jobs(namespace,kind,dedupe_key,payload,created_at) VALUES($1,'extract',$2,'{}',clock_timestamp()) RETURNING id").bind(bank).bind(Uuid::new_v4().to_string()).fetch_one(&store.pool).await.unwrap();
        ids.push(id);
    }
    let (one, two) = tokio::join!(store.claim_job(&["extract"]), store.claim_job(&["extract"]));
    let one = one.unwrap().unwrap();
    let two = two.unwrap().unwrap();
    assert_ne!(one.namespace, two.namespace);
    assert!([one.id, two.id].contains(&ids[0]));
    assert!([one.id, two.id].contains(&ids[2]));
    assert!(store.claim_job(&["extract"]).await.unwrap().is_none());
    // Failure backoff must block the later episode in this bank.
    let first = if one.namespace == ns { &one } else { &two };
    let independent = if one.namespace == other { &one } else { &two };
    let mut tx = store.pool.begin().await.unwrap();
    sqlx::query("UPDATE memory_jobs SET lease_until=now()-interval '1 minute' WHERE id=$1")
        .bind(first.id)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "UPDATE memory_namespace_leases SET lease_until=now()-interval '1 minute' WHERE job_id=$1",
    )
    .bind(first.id)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let recovered = store.claim_job(&["extract"]).await.unwrap().unwrap();
    assert_eq!(recovered.id, first.id);
    store
        .finish_job(first, Ok(json!({"stale":true})))
        .await
        .unwrap();
    assert_eq!(store.job(&ns, first.id).await.unwrap().status, "running");
    store
        .finish_job(&recovered, Err("transient provider failure".into()))
        .await
        .unwrap();
    store
        .finish_job(independent, Ok(json!({"done":true})))
        .await
        .unwrap();
    assert!(store.claim_job(&["extract"]).await.unwrap().is_none());
    sqlx::query("UPDATE memory_jobs SET available_at=now() WHERE id=$1")
        .bind(first.id)
        .execute(&store.pool)
        .await
        .unwrap();
    let retry = store.claim_job(&["extract"]).await.unwrap().unwrap();
    assert_eq!(retry.id, ids[0]);
    store
        .finish_job(&retry, Ok(json!({"done":true})))
        .await
        .unwrap();
    let next = store.claim_job(&["extract"]).await.unwrap().unwrap();
    assert_eq!(next.id, ids[1]);
    store
        .finish_job(&next, Ok(json!({"done":true})))
        .await
        .unwrap();
}

#[tokio::test]
async fn retroactive_correction_closes_old_belief_without_negative_interval() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    let store = Store::new(PgPoolOptions::new().connect(&url).await.unwrap());
    store.migrate().await.unwrap();
    let ns = format!("retroactive-{}", Uuid::new_v4());
    let at = Utc::now();
    let (_, original) = extract(
        &store,
        &ns,
        "Ada uses Python",
        at,
        vec![claim("Python", Cardinality::Single, false)],
    )
    .await;
    let old: Uuid =
        serde_json::from_value(original["decisions"][0]["assertion_id"].clone()).unwrap();
    let mut correction = claim("Rust", Cardinality::Single, true);
    correction.valid_from = Some(at - Duration::days(1));
    let (_, result) = extract(
        &store,
        &ns,
        "Ada uses Rust",
        at + Duration::seconds(1),
        vec![correction],
    )
    .await;
    let new: Uuid = serde_json::from_value(result["decisions"][0]["assertion_id"].clone()).unwrap();
    let old = store.assertion(&ns, old).await.unwrap();
    assert_eq!(old.status, "superseded");
    assert_eq!(old.valid_to, Some(old.valid_from));
    assert_eq!(
        store
            .assertion(&ns, new)
            .await
            .unwrap()
            .valid_from
            .timestamp_micros(),
        (at - Duration::days(1)).timestamp_micros()
    );
}
struct ConstantEmbedder;
#[async_trait]
impl Embedder for ConstantEmbedder {
    async fn embed(&self, _: &str) -> Result<Vec<f32>, AppError> {
        Ok(vec![0.5; 1024])
    }
    async fn ready(&self) -> bool {
        true
    }
    fn version(&self) -> &str {
        "constant"
    }
}
#[tokio::test]
async fn unprocessed_evidence_is_recalled_before_extraction_and_failures_are_reported() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; integration skipped");
        return;
    };
    let store = Store::new(
        PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .unwrap(),
    );
    store.migrate().await.unwrap();
    let ns = format!("pending-{}", Uuid::new_v4());
    let state = AppState {
        store: store.clone(),
        graph: None,
        embedder: Arc::new(ConstantEmbedder),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
    };
    let ready = READY_EXTRACT_JOBS.read().await;
    let (episode, _, _) = store
        .retain(&RetainRequest {
            namespace: ns.clone(),
            session_id: "session".into(),
            external_id: "pending-episode".into(),
            metadata: json!({}),
            events: vec![EvidenceEvent {
                role: "user".into(),
                content: "Remember that my side project is codenamed Marmalade and I deploy it on Fridays".into(),
                occurred_at: Utc::now(),
                metadata: json!({}),
            }],
        })
        .await
        .unwrap();
    drop(ready);
    let ask = |query: &str| -> RecallRequest {
        serde_json::from_value(json!({"namespace":ns,"query":query,"graph":false,"temporal":false}))
            .unwrap()
    };
    // The question shares only some words with the stored sentence, and no worker has run.
    let natural = recall_engine(
        &state,
        ask("What is the codename of my side project, and which day do I deploy it?"),
    )
    .await
    .unwrap();
    assert_eq!(natural.results[0].kind, "source_chunk");
    assert!(natural.results[0].statement.contains("Marmalade"));
    assert!(natural.results[0].ranks.contains_key("raw_fallback"));
    // Once the chunk has a vector, a question with no shared words still finds it.
    let evidence = store.episode_evidence(&ns, episode).await.unwrap();
    memory_engine::worker::embed_missing_chunks(&state, &evidence)
        .await
        .unwrap();
    let paraphrase = recall_engine(&state, ask("zzz unrelated phrasing"))
        .await
        .unwrap();
    assert!(
        paraphrase.results[0]
            .ranks
            .contains_key("raw_fallback_semantic")
    );
    // A failed extraction is reported, and the evidence is still served as raw text.
    sqlx::query("UPDATE memory_jobs SET status='failed',error='fixture' WHERE namespace=$1")
        .bind(&ns)
        .execute(&store.pool)
        .await
        .unwrap();
    let degraded = recall_engine(&state, ask("codename")).await.unwrap();
    assert!(
        degraded
            .degraded_reasons
            .iter()
            .any(|r| r.contains("extraction jobs failed"))
    );
    assert_eq!(degraded.results[0].kind, "source_chunk");
}
#[tokio::test]
async fn include_raw_keeps_recent_source_text_in_an_interpreted_namespace() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; integration skipped");
        return;
    };
    let store = Store::new(
        PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .unwrap(),
    );
    store.migrate().await.unwrap();
    let ns = format!("raw-{}", Uuid::new_v4());
    let state = AppState {
        store: store.clone(),
        graph: None,
        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
    };
    // The extractor kept one fact and missed the deployment day stated in the same message.
    extract(
        &store,
        &ns,
        "Ada uses Python and now deploys on Tuesdays",
        Utc::now(),
        vec![claim("Python", Cardinality::Single, false)],
    )
    .await;
    sqlx::query("UPDATE memory_jobs SET status='succeeded' WHERE namespace=$1 AND kind='extract'")
        .bind(&ns)
        .execute(&store.pool)
        .await
        .unwrap();
    let ask = |include_raw: bool| -> RecallRequest {
        serde_json::from_value(json!({"namespace":ns,"query":"Ada Python","graph":false,"temporal":false,"include_raw":include_raw})).unwrap()
    };
    let plain = recall_engine(&state, ask(false)).await.unwrap();
    assert!(!plain.results.is_empty());
    assert!(plain.results.iter().all(|hit| hit.kind != "source_chunk"));
    let with_raw = recall_engine(&state, ask(true)).await.unwrap();
    assert!(
        with_raw
            .results
            .iter()
            .any(|hit| hit.kind == "source_chunk" && hit.statement.contains("Tuesdays"))
    );
    assert!(
        with_raw
            .results
            .iter()
            .any(|hit| hit.kind != "source_chunk")
    );
}
#[tokio::test]
async fn requests_without_include_raw_keep_all_words_semantics_when_nothing_matched() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; integration skipped");
        return;
    };
    let store = Store::new(
        PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .unwrap(),
    );
    store.migrate().await.unwrap();
    let ns = format!("allwords-{}", Uuid::new_v4());
    let state = AppState {
        store: store.clone(),
        graph: None,
        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
    };
    // Extraction finished and produced no assertion, so only the raw fallback can answer.
    let ready = READY_EXTRACT_JOBS.read().await;
    store
        .retain(&RetainRequest {
            namespace: ns.clone(),
            session_id: "session".into(),
            external_id: "settled".into(),
            metadata: json!({}),
            events: vec![EvidenceEvent {
                role: "user".into(),
                content: "Ada deploys the service on Tuesdays".into(),
                occurred_at: Utc::now(),
                metadata: json!({}),
            }],
        })
        .await
        .unwrap();
    sqlx::query("UPDATE memory_jobs SET status='succeeded' WHERE namespace=$1 AND kind='extract'")
        .bind(&ns)
        .execute(&store.pool)
        .await
        .unwrap();
    drop(ready);
    let ask = |include_raw: bool| -> RecallRequest {
        serde_json::from_value(json!({"namespace":ns,"query":"What day does Ada deploy the service?","graph":false,"temporal":false,"include_raw":include_raw})).unwrap()
    };
    // "day" is not in the stored sentence: the all-words query every request used before matches nothing.
    let plain = recall_engine(&state, ask(false)).await.unwrap();
    assert!(plain.results.is_empty(), "{:?}", plain.results);
    let loose = recall_engine(&state, ask(true)).await.unwrap();
    assert!(
        loose
            .results
            .iter()
            .any(|h| h.statement.contains("Tuesdays"))
    );
}
struct Counting(std::sync::atomic::AtomicUsize);
#[async_trait]
impl JsonModel for Counting {
    async fn generate(&self, _: &str) -> Result<Value, AppError> {
        Ok(json!({"call":self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst)}))
    }
    fn identity(&self) -> String {
        "counting-fixture".into()
    }
}
#[tokio::test]
async fn forgotten_model_replies_are_asked_again_and_kept_ones_replay() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; integration skipped");
        return;
    };
    let store = Store::new(
        PgPoolOptions::new()
            .max_connections(2)
            .connect(&url)
            .await
            .unwrap(),
    );
    store.migrate().await.unwrap();
    let model = Counting(Default::default());
    let prompt = format!("prompt-{}", Uuid::new_v4());
    let first = memory_engine::model::cached_generate(&store, &model, "v", &prompt)
        .await
        .unwrap();
    let replay = memory_engine::model::cached_generate(&store, &model, "v", &prompt)
        .await
        .unwrap();
    assert_eq!(first, replay);
    memory_engine::model::forget_generated(&store, &model, "v", &prompt)
        .await
        .unwrap();
    let again = memory_engine::model::cached_generate(&store, &model, "v", &prompt)
        .await
        .unwrap();
    assert_ne!(first, again);
}

/// Embeds the texts of a table and nothing else, so a test fixes every distance. A text that is
/// not in the table is an error, never an invented vector.
struct TableEmbedder(HashMap<String, Vec<f32>>);
#[async_trait]
impl Embedder for TableEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, AppError> {
        self.0
            .get(text)
            .cloned()
            .ok_or_else(|| AppError::Provider(format!("no fixed vector for {text:?}")))
    }
    async fn ready(&self) -> bool {
        true
    }
    fn version(&self) -> &str {
        "table"
    }
}
/// A vector whose cosine distance from the query vector (the unit vector of `axis`) is
/// `distance`. Each fixture takes its own axis and the plane of the next one, so rows left by
/// other fixtures in a shared database are at distance 1 and never crowd out these rows in an
/// index scan.
fn at_distance(axis: usize, distance: f32) -> Vec<f32> {
    let cosine = 1.0 - distance;
    let mut vector = vec![0.0; 1024];
    vector[axis] = cosine;
    vector[axis + 1] = (1.0 - cosine * cosine).max(0.0).sqrt();
    vector
}
const FACT_QUERY: &str = "zzquux relevance probe";
const IDENTIFIER_QUERY: &str = "atlas-staging";
const RAW_QUERY: &str = "mobile release branch name";
const RAW_ONLY_QUERY: &str = "mobile release";
/// A fact the extractor kept, with its vector at `distance` from the query vector.
async fn embedded_fact(
    store: &Store,
    ns: &str,
    axis: usize,
    statement: &str,
    distance: f32,
) -> Uuid {
    let mut fact = claim(statement, Cardinality::Multiple, false);
    fact.statement = statement.into();
    fact.quotes = vec![statement.into()];
    let (_, result) = extract(store, ns, statement, Utc::now(), vec![fact]).await;
    let id: Uuid = serde_json::from_value(result["decisions"][0]["assertion_id"].clone()).unwrap();
    sqlx::query("UPDATE assertions SET embedding=$2 WHERE id=$1")
        .bind(id)
        .bind(Vector::from(at_distance(axis, distance)))
        .execute(&store.pool)
        .await
        .unwrap();
    id
}
/// Retained source text, with a vector at `distance` from the query vector or none yet.
async fn embedded_chunk(
    store: &Store,
    ns: &str,
    axis: usize,
    text: &str,
    distance: Option<f32>,
) -> Uuid {
    let (episode, _) = retain_one(store, ns, text, "user", text, Utc::now()).await;
    let chunk = store.episode_evidence(ns, episode).await.unwrap()[0].0;
    if let Some(distance) = distance {
        sqlx::query("UPDATE chunks SET embedding=$2 WHERE id=$1")
            .bind(chunk)
            .bind(Vector::from(at_distance(axis, distance)))
            .execute(&store.pool)
            .await
            .unwrap();
    }
    chunk
}
/// Two namespaces whose distances from the queries above are fixed by a table. `ns` holds five
/// facts at 0.30, 0.40, 0.60, 0.90 and 0.95. `raw_ns` holds retained text and no fact: chunks at
/// 0.30, 0.35, 0.70 and 0.80, and one chunk with no vector yet. The first and the last two chunks
/// share words with `RAW_QUERY`, the others only match it by vector.
struct DistanceFixture {
    state: Arc<AppState>,
    axis: usize,
    ns: String,
    near_fact: Uuid,
    mid_fact: Uuid,
    far_fact: Uuid,
    farther_fact: Uuid,
    identifier_fact: Uuid,
    raw_ns: String,
    word_and_near: Uuid,
    near_only: Uuid,
    far_only: Uuid,
    word_and_far: Uuid,
    word_without_vector: Uuid,
}
impl DistanceFixture {
    async fn new(test: &str) -> Option<Self> {
        let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
            eprintln!("TEST_DATABASE_URL missing; {test} skipped");
            return None;
        };
        let store = Store::new(
            PgPoolOptions::new()
                .max_connections(4)
                .connect(&url)
                .await
                .unwrap(),
        );
        store.migrate().await.unwrap();
        let axis = (Uuid::new_v4().as_u128() % 1000) as usize;
        let table = [FACT_QUERY, IDENTIFIER_QUERY, RAW_QUERY, RAW_ONLY_QUERY]
            .into_iter()
            .map(|query| (query.to_string(), at_distance(axis, 0.0)))
            .collect();
        let state = Arc::new(AppState {
            store: store.clone(),
            graph: None,
            embedder: Arc::new(TableEmbedder(table)),
            model: Arc::new(NoModel),
            worker_concurrency: 1,
        });
        let ns = format!("distance-facts-{}", Uuid::new_v4());
        let near_fact = embedded_fact(
            &store,
            &ns,
            axis,
            "Ada ships the mobile app from harbor-9",
            0.30,
        )
        .await;
        let mid_fact = embedded_fact(
            &store,
            &ns,
            axis,
            "Ada reviews pull requests with Meera",
            0.40,
        )
        .await;
        let far_fact = embedded_fact(
            &store,
            &ns,
            axis,
            "Ada edits source files in the Slate theme",
            0.60,
        )
        .await;
        let farther_fact =
            embedded_fact(&store, &ns, axis, "Ada drinks coffee at standup", 0.90).await;
        let identifier_fact = embedded_fact(
            &store,
            &ns,
            axis,
            "Ada deploys the preview to atlas-staging",
            0.95,
        )
        .await;
        let raw_ns = format!("distance-raw-{}", Uuid::new_v4());
        let word_and_near = embedded_chunk(
            &store,
            &raw_ns,
            axis,
            "The mobile release branch is harbor-9",
            Some(0.30),
        )
        .await;
        let near_only = embedded_chunk(
            &store,
            &raw_ns,
            axis,
            "Shipping the phone build from the harbor line",
            Some(0.35),
        )
        .await;
        let far_only = embedded_chunk(&store, &raw_ns, axis, "Lunch is at noon", Some(0.70)).await;
        let word_and_far = embedded_chunk(
            &store,
            &raw_ns,
            axis,
            "The mobile release checklist is long",
            Some(0.80),
        )
        .await;
        let word_without_vector = embedded_chunk(
            &store,
            &raw_ns,
            axis,
            "Mobile release freeze starts Monday",
            None,
        )
        .await;
        // Extraction has finished for all of it, so only the retained text searches add candidates.
        for namespace in [&ns, &raw_ns] {
            sqlx::query(
                "UPDATE memory_jobs SET status='succeeded' WHERE namespace=$1 AND kind='extract'",
            )
            .bind(namespace)
            .execute(&store.pool)
            .await
            .unwrap();
        }
        Some(Self {
            state,
            axis,
            ns,
            near_fact,
            mid_fact,
            far_fact,
            farther_fact,
            identifier_fact,
            raw_ns,
            word_and_near,
            near_only,
            far_only,
            word_and_far,
            word_without_vector,
        })
    }
    fn ask(&self, namespace: &str, query: &str, extra: Value) -> RecallRequest {
        let mut body = json!({"namespace":namespace,"query":query,"graph":false,"temporal":false});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value(body).unwrap()
    }
    async fn recall(&self, namespace: &str, query: &str, extra: Value) -> RecallResponse {
        recall_engine(&self.state, self.ask(namespace, query, extra))
            .await
            .unwrap()
    }
}
/// The ids a strategy ranked, best first.
fn ranked(response: &RecallResponse, strategy: &str) -> Vec<Uuid> {
    let mut hits: Vec<(usize, Uuid)> = response
        .ranking
        .iter()
        .filter_map(|hit| hit.ranks.get(strategy).map(|rank| (*rank, hit.id)))
        .collect();
    hits.sort();
    hits.into_iter().map(|(_, id)| id).collect()
}
fn ranking_ids(response: &RecallResponse) -> HashSet<Uuid> {
    response.ranking.iter().map(|hit| hit.id).collect()
}
async fn post_json(app: &axum::Router, uri: &str, body: &Value) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    (status, body)
}
#[tokio::test]
async fn max_distance_drops_far_facts_from_the_semantic_channel() {
    let Some(f) = DistanceFixture::new("max_distance semantic channel").await else {
        return;
    };
    // Nothing shares a word with the question, so only the vector channel contributes.
    let all = f.recall(&f.ns, FACT_QUERY, json!({})).await;
    assert_eq!(
        ranked(&all, "semantic"),
        [
            f.near_fact,
            f.mid_fact,
            f.far_fact,
            f.farther_fact,
            f.identifier_fact
        ]
    );
    assert!(ranked(&all, "lexical").is_empty());
    let near = f
        .recall(&f.ns, FACT_QUERY, json!({"max_distance":0.45}))
        .await;
    assert_eq!(ranked(&near, "semantic"), [f.near_fact, f.mid_fact]);
    // The survivors keep consecutive ranks, so fusion sees no gaps.
    let ranks: Vec<usize> = near.ranking.iter().map(|h| h.ranks["semantic"]).collect();
    assert_eq!(ranks.iter().copied().max(), Some(2));
    assert_eq!(ranking_ids(&near), HashSet::from([f.near_fact, f.mid_fact]));
    assert_eq!(
        near.results.iter().map(|h| h.id).collect::<HashSet<_>>(),
        HashSet::from([f.near_fact, f.mid_fact])
    );
    assert!(near.context.contains(&f.near_fact.to_string()));
    assert!(!near.context.contains(&f.far_fact.to_string()));
    // The cutoff is the request value: a tighter one keeps fewer, the widest keeps all.
    let tight = f
        .recall(&f.ns, FACT_QUERY, json!({"max_distance":0.35}))
        .await;
    assert_eq!(ranked(&tight, "semantic"), [f.near_fact]);
    let widest = f
        .recall(&f.ns, FACT_QUERY, json!({"max_distance":2.0}))
        .await;
    assert_eq!(ranked(&widest, "semantic"), ranked(&all, "semantic"));
}
#[tokio::test]
async fn max_distance_keeps_exact_word_matches_on_facts_however_far_their_vector_is() {
    let Some(f) = DistanceFixture::new("max_distance exact identifier").await else {
        return;
    };
    // "atlas-staging" is an identifier in one fact whose vector is the farthest of all.
    let all = f.recall(&f.ns, IDENTIFIER_QUERY, json!({})).await;
    assert_eq!(ranked(&all, "lexical"), [f.identifier_fact]);
    assert_eq!(ranked(&all, "semantic").last(), Some(&f.identifier_fact));
    for max_distance in [0.45, 0.05] {
        let gated = f
            .recall(
                &f.ns,
                IDENTIFIER_QUERY,
                json!({"max_distance":max_distance}),
            )
            .await;
        assert_eq!(ranked(&gated, "lexical"), [f.identifier_fact]);
        assert!(!ranked(&gated, "semantic").contains(&f.identifier_fact));
        let hit = gated
            .results
            .iter()
            .find(|h| h.id == f.identifier_fact)
            .unwrap_or_else(|| panic!("exact match dropped at {max_distance}"));
        assert_eq!(hit.ranks.keys().collect::<Vec<_>>(), ["lexical"]);
    }
    // Tighter than every vector: the exact match is all that is left.
    let tightest = f
        .recall(&f.ns, IDENTIFIER_QUERY, json!({"max_distance":0.05}))
        .await;
    assert_eq!(ranking_ids(&tightest), HashSet::from([f.identifier_fact]));
}
#[tokio::test]
async fn max_distance_drops_far_retained_text_and_keeps_text_without_a_vector() {
    let Some(f) = DistanceFixture::new("max_distance retained text").await else {
        return;
    };
    // include_raw adds an OR-of-words pass and a nearest-vectors pass over retained text.
    let all = f
        .recall(&f.raw_ns, RAW_QUERY, json!({"include_raw":true}))
        .await;
    assert_eq!(
        ranked(&all, "raw_fallback_semantic"),
        [f.word_and_near, f.near_only, f.far_only, f.word_and_far]
    );
    assert_eq!(
        ranked(&all, "raw_fallback")
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([f.word_and_near, f.word_and_far, f.word_without_vector])
    );
    let gated = f
        .recall(
            &f.raw_ns,
            RAW_QUERY,
            json!({"include_raw":true,"max_distance":0.45}),
        )
        .await;
    // The nearest-vectors pass lost the two far chunks.
    assert_eq!(
        ranked(&gated, "raw_fallback_semantic"),
        [f.word_and_near, f.near_only]
    );
    // The word pass lost the chunk whose vector is far and kept the one with no vector.
    assert_eq!(
        ranked(&gated, "raw_fallback")
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([f.word_and_near, f.word_without_vector])
    );
    assert_eq!(
        ranking_ids(&gated),
        HashSet::from([f.word_and_near, f.near_only, f.word_without_vector])
    );
    assert!(gated.ranking.iter().all(|hit| hit.kind == "source_chunk"));
    // A chunk with no vector has no distance, so even the tightest bound keeps it.
    let tightest = f
        .recall(
            &f.raw_ns,
            RAW_QUERY,
            json!({"include_raw":true,"max_distance":0.01}),
        )
        .await;
    assert_eq!(
        ranking_ids(&tightest),
        HashSet::from([f.word_without_vector])
    );
    // Requests for source text only gate their vector channel. Their word channel is the
    // exact all-words search every request has, so it is not gated.
    let raw_all = f
        .recall(&f.raw_ns, RAW_ONLY_QUERY, json!({"raw_only":true}))
        .await;
    assert_eq!(
        ranked(&raw_all, "semantic"),
        [f.word_and_near, f.near_only, f.far_only, f.word_and_far]
    );
    let raw_gated = f
        .recall(
            &f.raw_ns,
            RAW_ONLY_QUERY,
            json!({"raw_only":true,"max_distance":0.45}),
        )
        .await;
    assert_eq!(
        ranked(&raw_gated, "semantic"),
        [f.word_and_near, f.near_only]
    );
    assert_eq!(
        ranked(&raw_gated, "lexical")
            .into_iter()
            .collect::<HashSet<_>>(),
        ranked(&raw_all, "lexical")
            .into_iter()
            .collect::<HashSet<_>>()
    );
    assert_eq!(
        ranked(&raw_gated, "lexical")
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([f.word_and_near, f.word_and_far, f.word_without_vector])
    );
}
#[tokio::test]
async fn max_distance_without_a_query_vector_leaves_out_what_it_cannot_measure_and_says_so() {
    let Some(f) = DistanceFixture::new("max_distance without an embedder").await else {
        return;
    };
    let offline = AppState {
        store: f.state.store.clone(),
        graph: None,
        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
    };
    let ask = |namespace: &str, query: &str, extra: Value| f.ask(namespace, query, extra);
    // Without the field the offline request is what it always was: every word match.
    let unbounded = recall_engine(
        &offline,
        ask(&f.raw_ns, RAW_QUERY, json!({"include_raw":true})),
    )
    .await
    .unwrap();
    assert_eq!(
        ranking_ids(&unbounded),
        HashSet::from([f.word_and_near, f.word_and_far, f.word_without_vector])
    );
    let response = recall_engine(
        &offline,
        ask(
            &f.raw_ns,
            RAW_QUERY,
            json!({"include_raw":true,"max_distance":0.45}),
        ),
    )
    .await
    .unwrap();
    // No query vector means no distance. A chunk that has a vector cannot be shown to be near
    // enough, so it is left out instead of flooding the context with the ungated text, and
    // only the chunk that has no vector yet remains.
    assert_eq!(
        ranking_ids(&response),
        HashSet::from([f.word_without_vector])
    );
    assert!(
        response
            .degraded_reasons
            .iter()
            .any(|reason| reason.contains("semantic unavailable")),
        "{:?}",
        response.degraded_reasons
    );
    assert!(
        response
            .degraded_reasons
            .iter()
            .any(|reason| reason.contains("max_distance cannot be measured")),
        "{:?}",
        response.degraded_reasons
    );
    // The same for the nothing-matched pass of a request that did not ask for source text.
    let plain = recall_engine(
        &offline,
        ask(&f.raw_ns, RAW_ONLY_QUERY, json!({"max_distance":0.45})),
    )
    .await
    .unwrap();
    assert_eq!(ranking_ids(&plain), HashSet::from([f.word_without_vector]));
    // Exact word matches on facts need no vector, so the request still finds them.
    let identifier = recall_engine(
        &offline,
        ask(&f.ns, IDENTIFIER_QUERY, json!({"max_distance":0.45})),
    )
    .await
    .unwrap();
    assert_eq!(ranked(&identifier, "lexical"), [f.identifier_fact]);
    assert_eq!(ranking_ids(&identifier), HashSet::from([f.identifier_fact]));
    assert!(
        identifier
            .degraded_reasons
            .iter()
            .any(|reason| reason.contains("max_distance cannot be measured"))
    );
}
#[tokio::test]
async fn max_distance_leaves_an_empty_context_when_nothing_is_close_enough() {
    let Some(f) = DistanceFixture::new("max_distance empty result").await else {
        return;
    };
    let app = api::router(f.state.clone());
    // The automatic recall of the Pi extension, with a bound tighter than every fact.
    let body = |max_distance: Value| {
        let mut body = json!({"namespace":f.ns,"query":FACT_QUERY,"graph":false,"temporal":false,"include_raw":true,"max_tokens":2048,"top_k":10});
        body["max_distance"] = max_distance;
        body
    };
    let (status, empty) = post_json(&app, "/api/v2/recall", &body(json!(0.05))).await;
    assert_eq!(status, StatusCode::OK, "{empty}");
    assert_eq!(empty["results"], json!([]));
    assert_eq!(empty["ranking"], json!([]));
    assert_eq!(empty["context"], "");
    assert_eq!(empty["estimated_tokens"], 0);
    assert_eq!(empty["degraded_reasons"], json!([]));
    // The same request without the bound finds the nearest facts, so the gate made it empty.
    let (status, unbounded) = post_json(&app, "/api/v2/recall", &body(Value::Null)).await;
    assert_eq!(status, StatusCode::OK, "{unbounded}");
    assert!(!unbounded["results"].as_array().unwrap().is_empty());
    assert!(!unbounded["context"].as_str().unwrap().is_empty());
    // Reflect shares the request, and reports insufficient evidence without asking the model.
    let (status, reflected) = post_json(&app, "/api/v2/reflect", &body(json!(0.05))).await;
    assert_eq!(status, StatusCode::OK, "{reflected}");
    assert_eq!(reflected["insufficient_evidence"], true);
}
#[tokio::test]
async fn max_distance_outside_zero_to_two_is_a_400_with_a_message() {
    let bad_values = [
        json!(0),
        json!(0.0),
        json!(-0.0),
        json!(-0.5),
        json!(2.01),
        json!(3),
        json!(1e39),
    ];
    // Recall rejects the value before it reads anything, so no database is needed.
    let app = api::router(view_state(offline_store(), None));
    for bad in &bad_values {
        let (status, body) = post_json(
            &app,
            "/api/v2/recall",
            &json!({"namespace":"ns","query":"q","max_distance":bad}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
        let message = body["error"].as_str().unwrap_or_default();
        assert!(message.contains("max_distance"), "{bad}: {body}");
    }
    // Reflect records a trace of its failure, so it needs the database to answer 400.
    if let Some(store) = graph_view_store("max_distance reflect validation").await {
        let app = api::router(view_state(store, None));
        for bad in &bad_values {
            let (status, body) = post_json(
                &app,
                "/api/v2/reflect",
                &json!({"namespace":"ns","query":"q","max_distance":bad}),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{bad}: {body}");
            let message = body["error"].as_str().unwrap_or_default();
            assert!(message.contains("max_distance"), "{bad}: {body}");
        }
    }
    // The field is documented in the OpenAPI document with its range.
    let (status, document) = get_json(&app, "/api-docs/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    let field = &document["components"]["schemas"]["RecallRequest"]["properties"]["max_distance"];
    assert!(field.is_object(), "{document}");
    assert_eq!(field["maximum"], 2.0, "{field}");
    assert!(field.to_string().contains("exclusiveMinimum"), "{field}");
    assert!(
        !document["components"]["schemas"]["RecallRequest"]["required"]
            .to_string()
            .contains("max_distance")
    );
    // JSON has no NaN or infinity, a caller of the engine can still pass them.
    for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut request: RecallRequest =
            serde_json::from_value(json!({"namespace":"ns","query":"q"})).unwrap();
        request.max_distance = Some(bad);
        let state = view_state(offline_store(), None);
        match recall_engine(&state, request).await {
            Err(AppError::Validation(message)) => assert!(message.contains("max_distance")),
            other => panic!("{bad} must be a validation error, got {other:?}"),
        }
    }
}
#[tokio::test]
async fn recall_without_max_distance_returns_the_candidates_it_always_did() {
    // The request does not carry the field, so stored traces and the benchmark are unchanged.
    let plain: RecallRequest =
        serde_json::from_value(json!({"namespace":"ns","query":"q"})).unwrap();
    assert_eq!(plain.max_distance, None);
    assert!(
        json!(plain).get("max_distance").is_none(),
        "{}",
        json!(plain)
    );
    let null: RecallRequest =
        serde_json::from_value(json!({"namespace":"ns","query":"q","max_distance":null})).unwrap();
    assert_eq!(null.max_distance, None);
    let Some(f) = DistanceFixture::new("recall without max_distance").await else {
        return;
    };
    // Candidate sets recorded from the distances the fixture sets, not from the code under test.
    let all = f.recall(&f.ns, FACT_QUERY, json!({})).await;
    assert_eq!(
        ranked(&all, "semantic"),
        [
            f.near_fact,
            f.mid_fact,
            f.far_fact,
            f.farther_fact,
            f.identifier_fact
        ]
    );
    assert_eq!(
        all.ranking.iter().map(|h| h.ranks["semantic"]).max(),
        Some(5)
    );
    let raw = f
        .recall(&f.raw_ns, RAW_QUERY, json!({"include_raw":true}))
        .await;
    assert_eq!(
        ranked(&raw, "raw_fallback_semantic"),
        [f.word_and_near, f.near_only, f.far_only, f.word_and_far]
    );
    assert_eq!(
        ranked(&raw, "raw_fallback")
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([f.word_and_near, f.word_and_far, f.word_without_vector])
    );
    assert_eq!(
        ranking_ids(&raw),
        HashSet::from([
            f.word_and_near,
            f.near_only,
            f.far_only,
            f.word_and_far,
            f.word_without_vector
        ])
    );
    let raw_only = f
        .recall(&f.raw_ns, RAW_ONLY_QUERY, json!({"raw_only":true}))
        .await;
    assert_eq!(
        ranked(&raw_only, "semantic"),
        [f.word_and_near, f.near_only, f.far_only, f.word_and_far]
    );
    // Every cosine distance is at most 2, so that bound drops nothing: the replies are the
    // same as without the field, ranks, scores, packed evidence and all.
    for (namespace, query, extra) in [
        (&f.ns, FACT_QUERY, json!({})),
        (&f.ns, IDENTIFIER_QUERY, json!({})),
        (&f.ns, FACT_QUERY, json!({"include_raw":true})),
        (&f.raw_ns, RAW_QUERY, json!({"include_raw":true})),
        (&f.raw_ns, RAW_ONLY_QUERY, json!({"raw_only":true})),
    ] {
        let without = f.recall(namespace, query, extra.clone()).await;
        let mut explicit_null = extra.clone();
        explicit_null["max_distance"] = Value::Null;
        let with_null = f.recall(namespace, query, explicit_null).await;
        let mut widest = extra.clone();
        widest["max_distance"] = json!(2.0);
        let with_widest = f.recall(namespace, query, widest).await;
        for other in [&with_null, &with_widest] {
            assert_eq!(
                json!(without.ranking),
                json!(other.ranking),
                "{query} {extra}"
            );
            assert_eq!(
                json!(without.results),
                json!(other.results),
                "{query} {extra}"
            );
            assert_eq!(without.context, other.context, "{query} {extra}");
            assert_eq!(without.degraded_reasons, other.degraded_reasons);
        }
    }
}

/// A namespace for the rank tests: two facts that match every word of `RAW_QUERY`, and chunks
/// that match more or fewer of its words, so the any-word pass of `include_raw` ranks the far
/// chunks above the near ones.
struct RankFixture {
    ns: String,
    fact_a: Uuid,
    fact_b: Uuid,
    far_top: Uuid,
    near_a: Uuid,
    far_mid: Uuid,
    near_b: Uuid,
    no_vector: Uuid,
}
impl RankFixture {
    async fn new(f: &DistanceFixture) -> Self {
        let store = &f.state.store;
        let ns = format!("distance-ranks-{}", Uuid::new_v4());
        let fact_a = embedded_fact(
            store,
            &ns,
            f.axis,
            "The mobile release branch name is harbor-9",
            0.30,
        )
        .await;
        let fact_b = embedded_fact(
            store,
            &ns,
            f.axis,
            "The mobile release branch name changed in March",
            0.35,
        )
        .await;
        let far_top = embedded_chunk(
            store,
            &ns,
            f.axis,
            "mobile release branch name mobile release branch name mobile release branch name",
            Some(0.80),
        )
        .await;
        let near_a = embedded_chunk(
            store,
            &ns,
            f.axis,
            "mobile release branch for the phone app",
            Some(0.30),
        )
        .await;
        let far_mid = embedded_chunk(
            store,
            &ns,
            f.axis,
            "mobile release schedule for the phone app",
            Some(0.70),
        )
        .await;
        let near_b = embedded_chunk(
            store,
            &ns,
            f.axis,
            "mobile notes for the phone app",
            Some(0.40),
        )
        .await;
        let no_vector =
            embedded_chunk(store, &ns, f.axis, "release notes for the phone app", None).await;
        sqlx::query(
            "UPDATE memory_jobs SET status='succeeded' WHERE namespace=$1 AND kind='extract'",
        )
        .bind(&ns)
        .execute(&store.pool)
        .await
        .unwrap();
        Self {
            ns,
            fact_a,
            fact_b,
            far_top,
            near_a,
            far_mid,
            near_b,
            no_vector,
        }
    }
}
#[tokio::test]
async fn max_distance_removes_candidates_without_reordering_the_rest() {
    let Some(f) = DistanceFixture::new("max_distance ranks").await else {
        return;
    };
    let r = RankFixture::new(&f).await;
    let unbounded = f
        .recall(&r.ns, RAW_QUERY, json!({"include_raw":true}))
        .await;
    // The setup is not vacuous: in the any-word pass the far chunks rank above near ones, so
    // removing them would renumber every chunk behind them if ranks were taken over survivors.
    let words = ranked(&unbounded, "raw_fallback");
    let place = |id: Uuid| {
        words
            .iter()
            .position(|x| *x == id)
            .unwrap_or_else(|| panic!("{id} is not in the any-word pass: {words:?}"))
    };
    assert!(place(r.far_top) < place(r.near_a), "{words:?}");
    assert!(place(r.far_mid) < place(r.near_b), "{words:?}");
    assert!(place(r.far_top) < place(r.near_b), "{words:?}");
    assert!(ranked(&unbounded, "semantic").contains(&r.fact_a));
    let bounded = f
        .recall(
            &r.ns,
            RAW_QUERY,
            json!({"include_raw":true,"max_distance":0.45}),
        )
        .await;
    let kept = ranking_ids(&bounded);
    for far in [r.far_top, r.far_mid] {
        assert!(!kept.contains(&far), "{far} is farther than the bound");
    }
    for near in [r.fact_a, r.fact_b, r.near_a, r.near_b, r.no_vector] {
        assert!(kept.contains(&near), "{near} is within the bound");
    }
    // Every survivor keeps each rank it had without the bound, and no candidate is added.
    for hit in &bounded.ranking {
        let before = unbounded
            .ranking
            .iter()
            .find(|other| other.id == hit.id)
            .unwrap_or_else(|| panic!("the bound added {}", hit.statement));
        assert_eq!(hit.ranks, before.ranks, "{}", hit.statement);
        assert!(
            (hit.score - before.score).abs() < 1e-12,
            "{}",
            hit.statement
        );
    }
    // So the survivors come in the order they had: a chunk is never promoted over a fact.
    let before: Vec<Uuid> = unbounded
        .ranking
        .iter()
        .map(|hit| hit.id)
        .filter(|id| kept.contains(id))
        .collect();
    let after: Vec<Uuid> = bounded.ranking.iter().map(|hit| hit.id).collect();
    assert_eq!(after, before);
    // The any-word pass shows gaps where the far chunks were.
    let survivors: Vec<usize> = bounded
        .ranking
        .iter()
        .filter_map(|hit| hit.ranks.get("raw_fallback").copied())
        .collect();
    assert!(
        survivors.iter().max().unwrap() > &survivors.len(),
        "{survivors:?}"
    );
    // A bound that drops nothing changes nothing.
    let widest = f
        .recall(
            &r.ns,
            RAW_QUERY,
            json!({"include_raw":true,"max_distance":2.0}),
        )
        .await;
    assert_eq!(json!(widest.ranking), json!(unbounded.ranking));
}
#[tokio::test]
async fn max_distance_also_gates_source_text_that_requests_without_include_raw_search() {
    let Some(f) = DistanceFixture::new("max_distance source text without include_raw").await else {
        return;
    };
    let store = &f.state.store;
    // Evidence whose extraction has not finished is searched as text, with or without
    // include_raw, next to a fact that matched.
    let ns = format!("distance-unprocessed-{}", Uuid::new_v4());
    embedded_fact(store, &ns, f.axis, "Ada ships the mobile app", 0.30).await;
    let near = embedded_chunk(
        store,
        &ns,
        f.axis,
        "The mobile release branch is harbor-9",
        Some(0.30),
    )
    .await;
    let near_by_vector = embedded_chunk(
        store,
        &ns,
        f.axis,
        "Shipping the phone build from the harbor line",
        Some(0.35),
    )
    .await;
    let far_by_vector = embedded_chunk(store, &ns, f.axis, "Lunch is at noon", Some(0.70)).await;
    let far = embedded_chunk(
        store,
        &ns,
        f.axis,
        "The mobile release checklist is long",
        Some(0.80),
    )
    .await;
    let fresh = embedded_chunk(
        store,
        &ns,
        f.axis,
        "Mobile release freeze starts Monday",
        None,
    )
    .await;
    let unbounded = f.recall(&ns, RAW_QUERY, json!({})).await;
    assert_eq!(
        ranked(&unbounded, "raw_fallback_semantic"),
        [near, near_by_vector, far_by_vector, far]
    );
    assert_eq!(
        ranked(&unbounded, "raw_fallback")
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([near, far, fresh])
    );
    let bounded = f.recall(&ns, RAW_QUERY, json!({"max_distance":0.45})).await;
    assert_eq!(
        ranked(&bounded, "raw_fallback_semantic"),
        [near, near_by_vector]
    );
    assert_eq!(
        ranked(&bounded, "raw_fallback")
            .into_iter()
            .collect::<HashSet<_>>(),
        HashSet::from([near, fresh])
    );
    sqlx::query("UPDATE memory_jobs SET status='succeeded' WHERE namespace=$1 AND kind='extract'")
        .bind(&ns)
        .execute(&store.pool)
        .await
        .unwrap();
    // When nothing else matched, the all-words text search runs, and it is bounded too.
    let unbounded = f.recall(&f.raw_ns, RAW_ONLY_QUERY, json!({})).await;
    assert_eq!(
        ranking_ids(&unbounded),
        HashSet::from([f.word_and_near, f.word_and_far, f.word_without_vector])
    );
    let bounded = f
        .recall(&f.raw_ns, RAW_ONLY_QUERY, json!({"max_distance":0.45}))
        .await;
    assert_eq!(
        ranking_ids(&bounded),
        HashSet::from([f.word_and_near, f.word_without_vector])
    );
}
#[tokio::test]
async fn max_distance_drops_facts_that_only_the_graph_reached_when_they_are_far() {
    let Some((store, graph)) = graph_view_services("max_distance graph expansion").await else {
        return;
    };
    let Some(f) = DistanceFixture::new("max_distance graph expansion").await else {
        return;
    };
    // The five facts of the fixture are all about Ada, so the graph connects every one of them
    // to the others, however far their vectors are from the question.
    for id in [
        f.near_fact,
        f.mid_fact,
        f.far_fact,
        f.farther_fact,
        f.identifier_fact,
    ] {
        let fact = store.assertion(&f.ns, id).await.unwrap();
        let ada = vec![json!({"id":fact.subject_id,"name":"Ada","entity_type":"person"})];
        graph.project_assertion(&fact, 1, ada).await.unwrap();
    }
    let state = Arc::new(AppState {
        store: store.clone(),
        graph: Some(Arc::new(graph.clone())),
        embedder: f.state.embedder.clone(),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
    });
    let ids = [
        f.near_fact,
        f.mid_fact,
        f.far_fact,
        f.farther_fact,
        f.identifier_fact,
    ];
    let ns = f.ns.clone();
    // The checks run in their own task so the projected nodes are removed even when one fails.
    let outcome = tokio::spawn(graph_distance_checks(state, ns.clone(), ids)).await;
    delete_projected_namespace(&graph, &ns).await;
    if let Err(error) = outcome {
        match error.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            Err(error) => panic!("graph distance checks did not finish: {error}"),
        }
    }
}
async fn graph_distance_checks(state: Arc<AppState>, ns: String, ids: [Uuid; 5]) {
    let [near, mid, far, farther, identifier] = ids;
    let ask = |query: &str, extra: Value| {
        let mut body = json!({"namespace":ns,"query":query,"graph":true,"temporal":false});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        serde_json::from_value::<RecallRequest>(body).unwrap()
    };
    let step = |trace: memory_engine::observability::OperationTrace| {
        trace
            .steps
            .into_iter()
            .find(|step| step.stage == "graph_expansion")
            .unwrap()
            .details
    };
    let ids_of = |details: &Value, key: &str| -> HashSet<Uuid> {
        details[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|id| id.as_str().unwrap().parse().unwrap())
            .collect()
    };
    // Unbounded, the five facts are the vector seeds of the graph, so it adds nothing to them.
    let unbounded = recall_engine(&state, ask(FACT_QUERY, json!({})))
        .await
        .unwrap();
    assert_eq!(ranking_ids(&unbounded), HashSet::from(ids));
    let details = step(state.store.trace(unbounded.trace_id).await.unwrap());
    // The unbounded trace has no key for the bound, so traces of existing requests do not change.
    assert!(details.get("dropped_by_max_distance").is_none());
    // Bounded, only the two near facts are seeds. The graph walks from them to the three facts
    // about the same person, whose vectors are far from the question, and the bound drops them.
    let bounded = recall_engine(&state, ask(FACT_QUERY, json!({"max_distance":0.45})))
        .await
        .unwrap();
    assert_eq!(ranking_ids(&bounded), HashSet::from([near, mid]));
    assert!(ranked(&bounded, "graph").is_empty());
    for gone in [far, farther, identifier] {
        assert!(!bounded.context.contains(&gone.to_string()));
    }
    let details = step(state.store.trace(bounded.trace_id).await.unwrap());
    assert_eq!(
        ids_of(&details, "dropped_by_max_distance"),
        HashSet::from([far, farther, identifier]),
        "{details}"
    );
    assert!(ids_of(&details, "candidate_ids").is_empty(), "{details}");
    // Without a query vector the graph facts that have a vector cannot be shown to be near.
    // The exact word match seeds the walk and stays, and its neighbours are left out.
    let offline = AppState {
        store: state.store.clone(),
        graph: state.graph.clone(),

        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
    };
    let blind = recall_engine(
        &offline,
        ask(IDENTIFIER_QUERY, json!({"max_distance":0.45})),
    )
    .await
    .unwrap();
    assert_eq!(ranking_ids(&blind), HashSet::from([identifier]));
    assert!(
        blind
            .degraded_reasons
            .iter()
            .any(|reason| reason.contains("max_distance cannot be measured")),
        "{:?}",
        blind.degraded_reasons
    );
    let details = step(state.store.trace(blind.trace_id).await.unwrap());
    assert_eq!(
        ids_of(&details, "dropped_by_max_distance"),
        HashSet::from([near, mid, far, farther]),
        "{details}"
    );
    // The same offline request without the bound reaches them through the graph.
    let open = recall_engine(&offline, ask(IDENTIFIER_QUERY, json!({})))
        .await
        .unwrap();
    assert_eq!(ranking_ids(&open), HashSet::from(ids));
}
#[tokio::test]
async fn recall_states_the_cutoff_it_used_and_keeps_the_exact_number_it_was_given() {
    let Some(f) = DistanceFixture::new("max_distance echo").await else {
        return;
    };
    let app = api::router(f.state.clone());
    let body = |extra: Value| {
        let mut body = json!({"namespace":f.ns,"query":FACT_QUERY,"graph":false,"temporal":false});
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        body
    };
    // The reply says which cutoff produced it, so a client can tell a service that ignores the
    // field (one that predates it) from one that applied it.
    let (status, bounded) =
        post_json(&app, "/api/v2/recall", &body(json!({"max_distance":0.45}))).await;
    assert_eq!(status, StatusCode::OK, "{bounded}");
    assert_eq!(bounded["max_distance"], json!(0.45));
    let (status, plain) = post_json(&app, "/api/v2/recall", &body(json!({}))).await;
    assert_eq!(status, StatusCode::OK, "{plain}");
    assert!(plain.get("max_distance").is_none(), "{plain}");
    // The stored trace holds the number the caller sent, not a widened copy of it.
    let trace_id: Uuid = bounded["trace_id"].as_str().unwrap().parse().unwrap();
    let trace = f.state.store.trace(trace_id).await.unwrap();
    assert_eq!(trace.request["max_distance"], json!(0.45));
    // The bound is the number sent: a fact at 0.4 is kept by 0.4 and dropped by anything below.
    let at = f
        .recall(&f.ns, FACT_QUERY, json!({"max_distance":0.4000001}))
        .await;
    assert_eq!(ranked(&at, "semantic"), [f.near_fact, f.mid_fact]);
    let below = f
        .recall(&f.ns, FACT_QUERY, json!({"max_distance":0.3999}))
        .await;
    assert_eq!(ranked(&below, "semantic"), [f.near_fact]);
    // A value a 32 bit float would round to zero is a small positive number, not an error.
    let (status, tiny) =
        post_json(&app, "/api/v2/recall", &body(json!({"max_distance":1e-50}))).await;
    assert_eq!(status, StatusCode::OK, "{tiny}");
    assert_eq!(tiny["results"], json!([]));
    // A value of the wrong JSON type is rejected by the request parser (422), and a number out
    // of range by the engine (400); both say what is wrong.
    let (status, wrong) = post_json(
        &app,
        "/api/v2/recall",
        &body(json!({"max_distance":"0.45"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{wrong}");
    assert!(wrong.to_string().contains("max_distance"), "{wrong}");
}

// Graph view endpoints: /graph, /api/v2/graph/projection and /api/v2/graph/memories.
async fn get_raw(app: &axum::Router, uri: &str) -> (StatusCode, Option<String>, Vec<u8>) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, content_type, bytes.to_vec())
}
async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, Value) {
    let (status, _, bytes) = get_raw(app, uri).await;
    let body = serde_json::from_slice(&bytes).unwrap_or_else(|e| {
        panic!(
            "GET {uri} returned a body that is not JSON ({e}): {}",
            String::from_utf8_lossy(&bytes)
        )
    });
    (status, body)
}
fn view_state(store: Store, graph: Option<GraphStore>) -> Arc<AppState> {
    Arc::new(AppState {
        store,
        graph: graph.map(Arc::new),
        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
    })
}
/// A store whose PostgreSQL server does not exist, for requests that must fail or be rejected
/// before any query is answered.
fn offline_store() -> Store {
    Store::new(
        PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(500))
            .connect_lazy("postgres://memory:memory@127.0.0.1:1/unreachable")
            .unwrap(),
    )
}
fn unreachable_graph() -> GraphStore {
    GraphStore::new(
        "http://127.0.0.1:1".into(),
        "neo4j".into(),
        "password".into(),
        None,
    )
}
#[tokio::test]
async fn graph_view_rejects_bad_requests_and_reports_unavailable_dependencies() {
    let app = api::router(view_state(offline_store(), Some(unreachable_graph())));
    for path in ["/api/v2/graph/projection", "/api/v2/graph/memories"] {
        for query in [
            "",
            "?namespace=",
            "?namespace=%20%20",
            "?limit=10",
            "?namespace=ns&limit=0",
            "?namespace=ns&limit=-1",
            "?namespace=ns&limit=abc",
            "?namespace=ns&limit=",
            "?namespace=ns&limit=2001",
        ] {
            let (status, body) = get_json(&app, &format!("{path}{query}")).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}{query}");
            assert!(
                body["error"].as_str().is_some_and(|e| !e.is_empty()),
                "{path}{query} must explain the rejection: {body}"
            );
        }
    }
    let (status, body) = get_json(&app, "/api/v2/graph/memories?namespace=ns&limit=1001").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    let (status, body) = get_json(&app, "/api/v2/graph/projection?namespace=ns&limit=2001").await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    // Neo4j cannot be reached: a 503 with the reason, never an empty graph.
    let (status, body) = get_json(&app, "/api/v2/graph/projection?namespace=ns").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body["error"].as_str().unwrap().contains("Neo4j"), "{body}");
    // PostgreSQL cannot be reached: a loud server error, never an empty list.
    let (status, body) = get_json(&app, "/api/v2/graph/memories?namespace=ns").await;
    assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
    assert!(
        body["error"]
            .as_str()
            .unwrap()
            .starts_with("database error"),
        "{body}"
    );
    // Without a configured Neo4j the endpoint says so.
    let unconfigured = api::router(view_state(offline_store(), None));
    let (status, body) = get_json(&unconfigured, "/api/v2/graph/projection?namespace=ns").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert!(body["error"].as_str().unwrap().contains("not configured"));
    // The viewer page is served as HTML and both endpoints are in the OpenAPI document.
    let (status, content_type, page) = get_raw(&app, "/graph").await;
    assert_eq!(status, StatusCode::OK);
    assert!(content_type.unwrap().starts_with("text/html"));
    assert!(!page.is_empty());
    let (status, document) = get_json(&app, "/api-docs/openapi.json").await;
    assert_eq!(status, StatusCode::OK);
    for path in ["/api/v2/graph/projection", "/api/v2/graph/memories"] {
        assert!(document["paths"][path]["get"].is_object(), "{path}");
    }
}
/// PostgreSQL is needed. Missing configuration is stated, as in the other integration tests.
async fn graph_view_store(test: &str) -> Option<Store> {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL is required; {test} skipped");
        return None;
    };
    let store = Store::new(
        PgPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .unwrap(),
    );
    store.migrate().await.unwrap();
    Some(store)
}
/// Both integration services are needed.
async fn graph_view_services(test: &str) -> Option<(Store, GraphStore)> {
    let Ok(neo4j) = std::env::var("TEST_NEO4J_URL") else {
        eprintln!("TEST_NEO4J_URL is required; {test} skipped");
        return None;
    };
    let store = graph_view_store(test).await?;
    let graph = GraphStore::new(neo4j, "neo4j".into(), "password".into(), None);
    graph.ensure_constraints().await.unwrap();
    Some((store, graph))
}
/// Projects every assertion of the namespace the way the project job does.
async fn project_namespace(store: &Store, graph: &GraphStore, ns: &str) {
    let ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM assertions WHERE namespace=$1 ORDER BY recorded_at,id")
            .bind(ns)
            .fetch_all(&store.pool)
            .await
            .unwrap();
    for id in ids {
        let assertion = store.assertion(ns, id).await.unwrap();
        let revision: i64 = sqlx::query_scalar("SELECT revision FROM assertions WHERE id=$1")
            .bind(id)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        let entities: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',en.id,'name',en.name,'entity_type',en.entity_type) FROM assertion_entities ae JOIN entities en ON en.id=ae.entity_id WHERE ae.assertion_id=$1").bind(id).fetch_all(&store.pool).await.unwrap();
        graph
            .project_assertion(&assertion, revision, entities)
            .await
            .unwrap();
    }
}
fn id_set(nodes: &[Value], kind: &str) -> HashSet<Uuid> {
    nodes
        .iter()
        .filter(|n| n["kind"] == kind)
        .map(|n| n["id"].as_str().unwrap().parse().unwrap())
        .collect()
}
/// Removes what a test projected for its unique namespace, so test runs do not keep growing
/// the Neo4j database that a running service also reads.
async fn delete_projected_namespace(graph: &GraphStore, ns: &str) {
    let params = json!({"ns":ns});
    graph
        .execute_batch(&[
            (
                "MATCH (n:Assertion {namespace:$ns}) DETACH DELETE n",
                params.clone(),
            ),
            (
                "MATCH (n:KnowledgeEntity {namespace:$ns}) DETACH DELETE n",
                params.clone(),
            ),
            (
                "MATCH (n:MemoryNamespace {namespace:$ns}) DETACH DELETE n",
                params,
            ),
        ])
        .await
        .unwrap();
}
#[tokio::test]
async fn graph_view_serves_the_neo4j_projection_and_the_source_text_of_a_namespace() {
    let Some((store, graph)) =
        graph_view_services("graph_view_serves_the_neo4j_projection_and_the_source_text").await
    else {
        return;
    };
    let ns = format!("graphview-{}", Uuid::new_v4());
    // The checks run in their own task so the projected nodes are removed even when one fails.
    let outcome = tokio::spawn(graph_view_checks(store, graph.clone(), ns.clone())).await;
    delete_projected_namespace(&graph, &ns).await;
    let remaining = graph
        .execute_cypher(
            "MATCH (n {namespace:$ns}) RETURN count(n)",
            json!({"ns":ns}),
        )
        .await
        .unwrap();
    assert_eq!(remaining["results"][0]["data"][0]["row"], json!([0]));
    if let Err(error) = outcome {
        match error.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            Err(error) => panic!("graph view checks did not finish: {error}"),
        }
    }
}
async fn graph_view_checks(store: Store, graph: GraphStore, ns: String) {
    let at = Utc::now();
    let (_, r) = extract(
        &store,
        &ns,
        "Ada uses Python",
        at,
        vec![claim("Python", Cardinality::Single, false)],
    )
    .await;
    let python: Uuid = serde_json::from_value(r["decisions"][0]["assertion_id"].clone()).unwrap();
    let mut additive = claim("SQL", Cardinality::Multiple, false);
    additive.predicate = "knows language".into();
    let (_, r) = extract(
        &store,
        &ns,
        "Ada uses SQL",
        at + Duration::seconds(1),
        vec![additive],
    )
    .await;
    let sql: Uuid = serde_json::from_value(r["decisions"][0]["assertion_id"].clone()).unwrap();
    let subject = store.assertion(&ns, python).await.unwrap().subject_id;
    let observation = consolidate(
        &store,
        &ns,
        subject,
        "has language experience",
        vec![python, sql],
    )
    .await;
    let (_, r) = extract(
        &store,
        &ns,
        "Ada uses Rust now instead of Python",
        at + Duration::seconds(2),
        vec![claim("Rust", Cardinality::Single, true)],
    )
    .await;
    let rust: Uuid = serde_json::from_value(r["decisions"][0]["assertion_id"].clone()).unwrap();
    // Retained but never extracted: an assistant message, a tool message and a long message.
    let retain_only = |external: &str, role: &str, content: String, offset: i64| RetainRequest {
        namespace: ns.clone(),
        session_id: "unextracted".into(),
        external_id: external.into(),
        metadata: json!({}),
        events: vec![EvidenceEvent {
            role: role.into(),
            content,
            occurred_at: at + Duration::seconds(offset),
            metadata: json!({}),
        }],
    };
    for request in [
        retain_only("pending", "assistant", "Ada mentioned chess".into(), 3),
        retain_only("tool", "tool", "tool output that is not a memory".into(), 4),
        retain_only("long", "user", "\u{e9}".repeat(700), 5),
    ] {
        let _ready = READY_EXTRACT_JOBS.read().await;
        store.retain(&request).await.unwrap();
    }
    project_namespace(&store, &graph, &ns).await;
    let app = api::router(view_state(store.clone(), Some(graph.clone())));
    let q = urlencode(&ns);

    // Projection: exactly what Neo4j holds for the namespace, without the namespace gate.
    let gates = graph
        .execute_cypher(
            "MATCH (g:MemoryNamespace {namespace:$ns}) RETURN count(g)",
            json!({"ns":ns}),
        )
        .await
        .unwrap();
    assert_eq!(gates["results"][0]["data"][0]["row"], json!([1]));
    let (status, body) = get_json(&app, &format!("/api/v2/graph/projection?namespace={q}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["namespace"], json!(ns));
    assert_eq!(body["source"], "neo4j");
    assert_eq!(body["truncated"], false);
    let nodes = body["nodes"].as_array().unwrap();
    let relationships = body["relationships"].as_array().unwrap();
    assert_eq!(body["counts"]["nodes"], nodes.len());
    assert_eq!(body["counts"]["relationships"], relationships.len());
    assert!(
        nodes
            .iter()
            .all(|n| n["kind"] == "assertion" || n["kind"] == "entity"),
        "only assertions and entities are nodes: {nodes:?}"
    );
    let pg_assertions: HashSet<Uuid> = [python, sql, observation, rust].into();
    assert_eq!(id_set(nodes, "assertion"), pg_assertions);
    let pg_entities: HashSet<Uuid> =
        sqlx::query_scalar("SELECT DISTINCT entity_id FROM assertion_entities WHERE namespace=$1")
            .bind(&ns)
            .fetch_all(&store.pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert!(pg_entities.contains(&subject));
    assert_eq!(id_set(nodes, "entity"), pg_entities);
    let node = |id: Uuid| {
        nodes
            .iter()
            .find(|n| n["id"] == json!(id))
            .unwrap_or_else(|| panic!("node {id} missing"))
    };
    assert_eq!(node(rust)["statement"], "Ada uses Rust");
    assert_eq!(node(rust)["name"], "Ada uses Rust");
    assert_eq!(node(rust)["status"], "active");
    assert_eq!(node(rust)["assertion_kind"], "fact");
    assert!(node(rust)["valid_from"].as_str().is_some());
    assert!(node(rust)["valid_to"].is_null());
    assert!(node(rust)["revision"].as_i64().is_some());
    assert_eq!(node(python)["status"], "superseded");
    assert!(node(python)["valid_to"].as_str().is_some());
    assert_eq!(node(observation)["assertion_kind"], "observation");
    assert_eq!(node(observation)["status"], "stale");
    assert_eq!(node(subject)["name"], "Ada");
    assert_eq!(node(subject)["entity_type"], "person");
    assert!(node(subject)["statement"].is_null());
    let mentions: HashSet<(Uuid, Uuid)> = relationships
        .iter()
        .filter(|r| r["type"] == "MENTIONS")
        .map(|r| {
            (
                r["from"].as_str().unwrap().parse().unwrap(),
                r["to"].as_str().unwrap().parse().unwrap(),
            )
        })
        .collect();
    let pg_mentions: HashSet<(Uuid, Uuid)> =
        sqlx::query_as("SELECT assertion_id,entity_id FROM assertion_entities WHERE namespace=$1")
            .bind(&ns)
            .fetch_all(&store.pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert!(!pg_mentions.is_empty());
    assert_eq!(mentions, pg_mentions);
    let supports: HashSet<(Uuid, Uuid, String)> = relationships
        .iter()
        .filter(|r| r["type"] == "SUPPORTS")
        .map(|r| {
            (
                r["from"].as_str().unwrap().parse().unwrap(),
                r["to"].as_str().unwrap().parse().unwrap(),
                r["relation"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let pg_supports: HashSet<(Uuid, Uuid, String)> =
        sqlx::query_as("SELECT from_id,to_id,relation FROM assertion_edges WHERE namespace=$1")
            .bind(&ns)
            .fetch_all(&store.pool)
            .await
            .unwrap()
            .into_iter()
            .collect();
    assert!(supports.contains(&(rust, python, "supersedes".to_string())));
    assert_eq!(supports, pg_supports);
    assert!(
        relationships
            .iter()
            .filter(|r| r["type"] == "SUPPORTS")
            .all(|r| r["explanation"].as_str().is_some())
    );
    assert_eq!(relationships.len(), mentions.len() + supports.len());

    // A node limit truncates openly and never returns a relationship with a missing end.
    let (status, small) = get_json(
        &app,
        &format!("/api/v2/graph/projection?namespace={q}&limit=3"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{small}");
    assert_eq!(small["truncated"], true);
    let small_nodes = small["nodes"].as_array().unwrap();
    assert_eq!(small_nodes.len(), 3);
    // A cut graph keeps a share of its nodes for the entities the facts mention, so the
    // MENTIONS relationships survive: two facts and the subject, not three facts alone.
    assert_eq!(id_set(small_nodes, "assertion").len(), 2, "{small}");
    assert_eq!(id_set(small_nodes, "entity"), HashSet::from([subject]));
    assert!(
        small["relationships"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["type"] == "MENTIONS"),
        "{small}"
    );
    let kept: HashSet<&str> = small_nodes
        .iter()
        .map(|n| n["id"].as_str().unwrap())
        .collect();
    assert!(small["relationships"].as_array().unwrap().iter().all(|r| {
        kept.contains(r["from"].as_str().unwrap()) && kept.contains(r["to"].as_str().unwrap())
    }));
    // Another namespace sees none of it.
    let other = urlencode(&format!("{ns}-other"));
    let (status, empty) =
        get_json(&app, &format!("/api/v2/graph/projection?namespace={other}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(empty["nodes"], json!([]));

    // Memories: cited chunks with their quotes, plus chunks extraction has not seen.
    let (status, memories) = get_json(&app, &format!("/api/v2/graph/memories?namespace={q}")).await;
    assert_eq!(status, StatusCode::OK, "{memories}");
    assert_eq!(memories["namespace"], json!(ns));
    assert_eq!(memories["source"], "postgres");
    assert_eq!(memories["truncated"], false);
    let list = memories["memories"].as_array().unwrap();
    // Python, SQL and Rust statements, the pending assistant message and the long message.
    assert_eq!(list.len(), 5, "{list:?}");
    assert_eq!(memories["counts"]["memories"], 5);
    assert_eq!(memories["counts"]["supported"], 3);
    // The exact PostgreSQL assertion count lets the viewer tell when the projection is behind,
    // by status too, since a status change leaves the total as it was.
    assert_eq!(memories["counts"]["assertions"], 4);
    assert_eq!(
        memories["counts"]["assertions_by_status"],
        json!({"active":2,"stale":1,"superseded":1})
    );
    // The three fixture memories are still queued for extraction, nothing has failed.
    let jobs = &memories["counts"]["jobs"];
    assert_eq!(jobs["extract_pending"], 3, "{jobs}");
    for idle in [
        "extract_running",
        "extract_failed",
        "extract_blocked",
        "project_running",
        "project_failed",
    ] {
        assert_eq!(jobs[idle], 0, "{idle}: {jobs}");
    }
    assert!(jobs["project_pending"].as_u64().unwrap() > 0, "{jobs}");
    assert!(
        list.iter().all(|m| m["role"] != "tool"),
        "an unextracted tool chunk is not a memory"
    );
    let by_text = |prefix: &str| {
        list.iter()
            .find(|m| m["text"].as_str().unwrap().starts_with(prefix))
            .unwrap_or_else(|| panic!("memory starting {prefix:?} missing"))
    };
    for (text, assertion, quote) in [
        ("Ada uses Python", python, "Ada uses Python"),
        ("Ada uses SQL", sql, "Ada uses SQL"),
        ("Ada uses Rust now", rust, "Ada uses Rust"),
    ] {
        let memory = by_text(text);
        assert_eq!(memory["role"], "user");
        assert_eq!(memory["processing_state"], "processed");
        assert_eq!(memory["extraction"]["status"], "succeeded");
        assert!(memory["extraction"]["error"].is_null());
        assert!(memory["session_id"].as_str().is_some());
        assert!(memory["occurred_at"].as_str().is_some());
        assert_eq!(memory["text_truncated"], false);
        // An observation also cites the chunks of the facts it combines.
        assert!(
            memory["supports"]
                .as_array()
                .unwrap()
                .contains(&json!({"assertion_id":assertion,"quote":quote})),
            "{memory}"
        );
    }
    // Every citation PostgreSQL holds for the namespace is on exactly the right chunk.
    let served: HashSet<(Uuid, Uuid, String)> = list
        .iter()
        .flat_map(|m| {
            m["supports"].as_array().unwrap().iter().map(|s| {
                (
                    m["id"].as_str().unwrap().parse().unwrap(),
                    s["assertion_id"].as_str().unwrap().parse().unwrap(),
                    s["quote"].as_str().unwrap().to_string(),
                )
            })
        })
        .collect();
    let stored: HashSet<(Uuid, Uuid, String)> = sqlx::query_as("SELECT s.chunk_id,s.assertion_id,s.quote FROM assertion_sources s JOIN assertions a ON a.id=s.assertion_id WHERE a.namespace=$1").bind(&ns).fetch_all(&store.pool).await.unwrap().into_iter().collect();
    assert!(stored.len() >= 4);
    assert_eq!(served, stored);
    let pending = by_text("Ada mentioned chess");
    assert_eq!(pending["role"], "assistant");
    assert_eq!(pending["processing_state"], "pending");
    assert_eq!(pending["extraction"]["status"], "pending");
    assert!(pending["extraction"]["error"].is_null());
    assert_eq!(pending["supports"], json!([]));
    let long = list
        .iter()
        .find(|m| m["text_truncated"] == true)
        .expect("the long message is flagged as truncated");
    assert_eq!(long["text"].as_str().unwrap().chars().count(), 600);
    assert_eq!(long["supports"], json!([]));
    // A limit says that it cut the rest. A quarter of it (at least one) is kept for chunks that
    // no fact cites, so the newest unextracted memory stays; cited chunks fill the rest, newest
    // first.
    let (status, cut) = get_json(
        &app,
        &format!("/api/v2/graph/memories?namespace={q}&limit=3"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{cut}");
    assert_eq!(cut["truncated"], true);
    let cut_list = cut["memories"].as_array().unwrap();
    assert_eq!(cut_list.len(), 3);
    let cut_text: Vec<&str> = cut_list
        .iter()
        .map(|m| m["text"].as_str().unwrap())
        .collect();
    assert!(
        cut_text.iter().any(|t| t.starts_with('\u{e9}')),
        "{cut_text:?}"
    );
    assert!(cut_text.contains(&"Ada uses Rust now instead of Python"));
    assert!(cut_text.contains(&"Ada uses SQL"));
    assert_eq!(cut["counts"]["memories"], 3);
    assert_eq!(cut["counts"]["supported"], 2);
    // The totals describe the namespace, not the page.
    assert_eq!(cut["counts"]["assertions"], 4);
    let (status, none) = get_json(&app, &format!("/api/v2/graph/memories?namespace={other}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(none["memories"], json!([]));
}
/// Retains one event that is never extracted by the test, and returns its episode and job.
async fn retain_one(
    store: &Store,
    ns: &str,
    external: &str,
    role: &str,
    content: &str,
    at: chrono::DateTime<Utc>,
) -> (Uuid, Uuid) {
    let _ready = READY_EXTRACT_JOBS.read().await;
    let (episode, job, _) = store
        .retain(&RetainRequest {
            namespace: ns.into(),
            session_id: "unextracted".into(),
            external_id: external.into(),
            metadata: json!({}),
            events: vec![EvidenceEvent {
                role: role.into(),
                content: content.into(),
                occurred_at: at,
                metadata: json!({}),
            }],
        })
        .await
        .unwrap();
    (episode, job)
}
fn memory_extraction<'a>(body: &'a Value, text: &str) -> &'a Value {
    &body["memories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["text"] == text)
        .unwrap_or_else(|| panic!("memory {text:?} missing: {body}"))["extraction"]
}
#[tokio::test]
async fn graph_view_tells_a_failed_extraction_from_a_waiting_one() {
    let Some(store) = graph_view_store("graph_view_tells_a_failed_extraction").await else {
        return;
    };
    let ns = format!("graphview-extraction-{}", Uuid::new_v4());
    let q = urlencode(&ns);
    let at = Utc::now();
    let app = api::router(view_state(store.clone(), None));
    let memories = || async {
        let (status, body) = get_json(&app, &format!("/api/v2/graph/memories?namespace={q}")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body
    };
    extract(
        &store,
        &ns,
        "Ada uses Python",
        at,
        vec![claim("Python", Cardinality::Single, false)],
    )
    .await;
    let (_, failing) = retain_one(
        &store,
        &ns,
        "failing",
        "user",
        "Ada has a note",
        at + Duration::seconds(1),
    )
    .await;
    retain_one(
        &store,
        &ns,
        "behind",
        "assistant",
        "Ada mentioned chess",
        at + Duration::seconds(2),
    )
    .await;
    let set_job = |status: &'static str, attempts: i32, error: Option<&'static str>| {
        let pool = store.pool.clone();
        async move {
            sqlx::query("UPDATE memory_jobs SET status=$2,attempts=$3,error=$4 WHERE id=$1")
                .bind(failing)
                .bind(status)
                .bind(attempts)
                .bind(error)
                .execute(&pool)
                .await
                .unwrap();
        }
    };

    // The worker gave up on the first retained episode. Its raw event still reads "pending",
    // and the episode behind it never runs, because the worker does not skip a failed job.
    set_job(
        "failed",
        5,
        Some("provider error: extraction failed validation"),
    )
    .await;
    let body = memories().await;
    let done = memory_extraction(&body, "Ada uses Python");
    assert_eq!(done["status"], "succeeded");
    assert!(done["error"].is_null());
    let failed = memory_extraction(&body, "Ada has a note");
    assert_eq!(failed["status"], "failed", "{failed}");
    assert_eq!(
        failed["error"],
        "provider error: extraction failed validation"
    );
    assert_eq!(failed["attempts"], 5);
    assert_eq!(failed["max_attempts"], 5);
    let blocked = memory_extraction(&body, "Ada mentioned chess");
    assert_eq!(blocked["status"], "blocked", "{blocked}");
    assert_eq!(blocked["error"], failed["error"]);
    for text in ["Ada has a note", "Ada mentioned chess"] {
        let memory = body["memories"]
            .as_array()
            .unwrap()
            .iter()
            .find(|m| m["text"] == text)
            .unwrap();
        assert_eq!(memory["processing_state"], "pending");
        assert_eq!(memory["supports"], json!([]));
    }
    let jobs = &body["counts"]["jobs"];
    assert_eq!(jobs["extract_failed"], 1, "{jobs}");
    assert_eq!(jobs["extract_blocked"], 1, "{jobs}");
    assert_eq!(jobs["extract_pending"], 0, "{jobs}");
    assert_eq!(jobs["extract_running"], 0, "{jobs}");

    // A retry that is waiting for its next attempt keeps the last error and blocks nothing.
    set_job("pending", 1, Some("timeout")).await;
    let body = memories().await;
    let retrying = memory_extraction(&body, "Ada has a note");
    assert_eq!(retrying["status"], "pending", "{retrying}");
    assert_eq!(retrying["error"], "timeout");
    assert_eq!(retrying["attempts"], 1);
    assert_eq!(
        memory_extraction(&body, "Ada mentioned chess")["status"],
        "pending"
    );
    assert_eq!(body["counts"]["jobs"]["extract_pending"], 2);
    assert_eq!(body["counts"]["jobs"]["extract_blocked"], 0);

    set_job("running", 1, None).await;
    let body = memories().await;
    assert_eq!(
        memory_extraction(&body, "Ada has a note")["status"],
        "running"
    );
    assert_eq!(body["counts"]["jobs"]["extract_running"], 1);
    assert_eq!(body["counts"]["jobs"]["extract_pending"], 1);
}
#[tokio::test]
async fn graph_view_keeps_the_newest_unextracted_memories_when_cited_ones_fill_the_limit() {
    let Some(store) = graph_view_store("graph_view_keeps_the_newest_unextracted_memories").await
    else {
        return;
    };
    let app = api::router(view_state(store.clone(), None));
    let at = Utc::now();
    let ns = format!("graphview-limit-{}", Uuid::new_v4());
    let q = urlencode(&ns);
    for i in 0..6 {
        extract(
            &store,
            &ns,
            &format!("Ada uses lang{i}"),
            at + Duration::seconds(i),
            vec![claim(&format!("lang{i}"), Cardinality::Multiple, false)],
        )
        .await;
    }
    retain_one(
        &store,
        &ns,
        "fresh",
        "assistant",
        "Fresh unprocessed note",
        at + Duration::seconds(10),
    )
    .await;
    let texts = |body: &Value| -> Vec<String> {
        body["memories"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["text"].as_str().unwrap().to_string())
            .collect()
    };
    // Six cited memories and one memory nothing cites yet. However small the limit, the page
    // keeps a share for the newest chunk nothing cites, and cited chunks take the rest.
    let (status, body) = get_json(
        &app,
        &format!("/api/v2/graph/memories?namespace={q}&limit=4"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["truncated"], true);
    assert_eq!(
        texts(&body),
        [
            "Fresh unprocessed note",
            "Ada uses lang5",
            "Ada uses lang4",
            "Ada uses lang3"
        ]
    );
    assert_eq!(body["counts"]["supported"], 3);
    assert_eq!(body["counts"]["assertions"], 6);
    let (_, body) = get_json(
        &app,
        &format!("/api/v2/graph/memories?namespace={q}&limit=2"),
    )
    .await;
    assert_eq!(texts(&body), ["Fresh unprocessed note", "Ada uses lang5"]);
    assert_eq!(body["truncated"], true);
    let (_, body) = get_json(
        &app,
        &format!("/api/v2/graph/memories?namespace={q}&limit=1"),
    )
    .await;
    assert_eq!(body["memories"].as_array().unwrap().len(), 1);
    assert_eq!(body["truncated"], true);
    let (_, body) = get_json(
        &app,
        &format!("/api/v2/graph/memories?namespace={q}&limit=7"),
    )
    .await;
    assert_eq!(body["memories"].as_array().unwrap().len(), 7);
    assert_eq!(body["truncated"], false);

    // Among chunks nothing cites, one that extraction has not processed beats a newer one
    // that was processed and produced no fact.
    let ns = format!("graphview-limit-{}", Uuid::new_v4());
    let q = urlencode(&ns);
    for i in 0..4 {
        extract(
            &store,
            &ns,
            &format!("Ada uses lang{i}"),
            at + Duration::seconds(i),
            vec![claim(&format!("lang{i}"), Cardinality::Multiple, false)],
        )
        .await;
    }
    retain_one(
        &store,
        &ns,
        "old-pending",
        "user",
        "Old but still unprocessed",
        at - Duration::seconds(100),
    )
    .await;
    retain_one(
        &store,
        &ns,
        "new-processed",
        "user",
        "Newer and processed without a fact",
        at + Duration::seconds(20),
    )
    .await;
    sqlx::query("UPDATE raw_events SET processing_state='processed' WHERE content=$1")
        .bind("Newer and processed without a fact")
        .execute(&store.pool)
        .await
        .unwrap();
    let (_, body) = get_json(
        &app,
        &format!("/api/v2/graph/memories?namespace={q}&limit=4"),
    )
    .await;
    assert_eq!(
        texts(&body),
        [
            "Ada uses lang3",
            "Ada uses lang2",
            "Ada uses lang1",
            "Old but still unprocessed"
        ]
    );
}
#[tokio::test]
async fn graph_view_namespace_with_nothing_returns_empty_lists() {
    let Some((store, graph)) =
        graph_view_services("graph_view_namespace_with_nothing_returns_empty_lists").await
    else {
        return;
    };
    let app = api::router(view_state(store, Some(graph)));
    let q = urlencode(&format!("graphview-empty-{}", Uuid::new_v4()));
    let (status, projection) =
        get_json(&app, &format!("/api/v2/graph/projection?namespace={q}")).await;
    assert_eq!(status, StatusCode::OK, "{projection}");
    assert_eq!(projection["source"], "neo4j");
    assert_eq!(projection["nodes"], json!([]));
    assert_eq!(projection["relationships"], json!([]));
    assert_eq!(projection["counts"], json!({"nodes":0,"relationships":0}));
    assert_eq!(projection["truncated"], false);
    let (status, memories) = get_json(&app, &format!("/api/v2/graph/memories?namespace={q}")).await;
    assert_eq!(status, StatusCode::OK, "{memories}");
    assert_eq!(memories["source"], "postgres");
    assert_eq!(memories["memories"], json!([]));
    assert_eq!(
        memories["counts"],
        json!({"memories":0,"supported":0,"assertions":0,"assertions_by_status":{},
            "jobs":{"extract_pending":0,"extract_running":0,"extract_failed":0,"extract_blocked":0,
                "project_pending":0,"project_running":0,"project_failed":0}})
    );
    assert_eq!(memories["truncated"], false);
}
fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Counts what one namespace still holds in PostgreSQL.
async fn namespace_rows(store: &Store, ns: &str) -> Value {
    let count = |sql: &'static str| {
        let ns = ns.to_string();
        let pool = store.pool.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .bind(ns)
                .fetch_one(&pool)
                .await
                .unwrap()
        }
    };
    json!({
        "assertions": count("SELECT count(*) FROM assertions WHERE namespace=$1").await,
        "entities": count("SELECT count(*) FROM entities WHERE namespace=$1").await,
        "episodes": count("SELECT count(*) FROM episodes WHERE namespace=$1").await,
        "episode_sources": count("SELECT count(*) FROM episode_sources es JOIN episodes e ON e.id=es.episode_id WHERE e.namespace=$1").await,
        "assertion_edges": count("SELECT count(*) FROM assertion_edges WHERE namespace=$1").await,
        "events": count("SELECT count(*) FROM raw_events re JOIN sessions s ON s.id=re.session_id WHERE s.namespace=$1").await,
        "chunks": count("SELECT count(*) FROM chunks c JOIN raw_events re ON re.id=c.raw_event_id JOIN sessions s ON s.id=re.session_id WHERE s.namespace=$1").await,
        "sessions": count("SELECT count(*) FROM sessions WHERE namespace=$1").await,
        "legacy_memories": count("SELECT count(*) FROM memories WHERE namespace=$1 || ':legacy'").await,
        "legacy_sources": count("SELECT count(*) FROM memory_version_sources mvs JOIN memory_versions mv ON mv.id=mvs.memory_version_id JOIN memories m ON m.id=mv.memory_id WHERE m.namespace=$1 || ':legacy'").await,
        "work_jobs": count("SELECT count(*) FROM memory_jobs WHERE namespace=$1 AND kind<>'clear_graph'").await,
        "clear_jobs": count("SELECT count(*) FROM memory_jobs WHERE namespace=$1 AND kind='clear_graph'").await,
    })
}
async fn neo4j_nodes(graph: &GraphStore, ns: &str) -> i64 {
    let rows = graph
        .execute_cypher(
            "MATCH (n {namespace:$ns}) WHERE NOT n:MemoryNamespace RETURN count(n)",
            json!({"ns":ns}),
        )
        .await
        .unwrap();
    rows["results"][0]["data"][0]["row"][0].as_i64().unwrap()
}
async fn clear_checks(store: Store, graph: GraphStore, ns: String, other: String, busy: String) {
    let at = Utc::now();
    // Two namespaces, each with a correction chain (Rust superseded by Go), projected to Neo4j.
    for namespace in [&ns, &other] {
        extract(
            &store,
            namespace,
            "Ada uses Rust",
            at,
            vec![claim("Rust", Cardinality::Single, false)],
        )
        .await;
        extract(
            &store,
            namespace,
            "Ada uses Go",
            at + Duration::seconds(1),
            vec![claim("Go", Cardinality::Single, true)],
        )
        .await;
        project_namespace(&store, &graph, namespace).await;
        // Older releases stored shadow memories that share these chunks. Verify cleanup during upgrades.
        let chunk: Uuid = sqlx::query_scalar("SELECT c.id FROM chunks c JOIN raw_events re ON re.id=c.raw_event_id JOIN sessions s ON s.id=re.session_id WHERE s.namespace=$1 ORDER BY c.created_at LIMIT 1")
            .bind(namespace)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        let memory: Uuid = sqlx::query_scalar("INSERT INTO memories(namespace,canonical_key,subject,predicate) VALUES($1 || ':legacy','ada::uses language','Ada','uses language') RETURNING id")
            .bind(namespace)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        let version: Uuid = sqlx::query_scalar("INSERT INTO memory_versions(memory_id,version,value,normalized_value,statement,kind,status,valid_from,extractor_version) VALUES($1,1,'Rust','rust','Ada uses Rust','fact','active',$2,'fixture') RETURNING id")
            .bind(memory)
            .bind(at)
            .fetch_one(&store.pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO memory_version_sources(memory_version_id,chunk_id) VALUES($1,$2)")
            .bind(version)
            .bind(chunk)
            .execute(&store.pool)
            .await
            .unwrap();
    }
    // A trace written by V1 ingest points at a session and an event of the namespace. Traces stay when the namespace is
    // cleared, so the clear has to unlink them and not trip over them.
    sqlx::query("INSERT INTO operations(id,operation_type,namespace,session_id,event_id,status,started_at,finished_at,duration_ms,request) SELECT gen_random_uuid(),'ingest',$1,s.id,re.id,'succeeded',now(),now(),1,'{}'::jsonb FROM sessions s JOIN raw_events re ON re.session_id=s.id WHERE s.namespace=$1 LIMIT 1")
        .bind(&ns)
        .execute(&store.pool)
        .await
        .unwrap();
    let before = namespace_rows(&store, &ns).await;
    assert_eq!(
        before["legacy_memories"], 1,
        "the shadow copy exists before the clear: {before}"
    );
    assert_eq!(before["legacy_sources"], 1);
    assert_eq!(before["assertions"], 2);
    assert_eq!(before["episodes"], 2);
    assert_eq!(before["events"], 2);
    assert_eq!(before["chunks"], 2);
    assert!(
        neo4j_nodes(&graph, &ns).await >= 3,
        "two facts and an entity are projected"
    );
    let other_before = namespace_rows(&store, &other).await;
    let other_nodes = neo4j_nodes(&graph, &other).await;
    let app = api::router(view_state(store.clone(), Some(graph.clone())));

    // The confirmation has to repeat the namespace, and a refused request deletes nothing.
    for confirm in ["", "yes", &format!("{ns} ")] {
        let (status, body) = post_json(
            &app,
            "/api/v2/graph/clear",
            &json!({"namespace":ns,"confirm":confirm}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body.to_string()
                .contains("confirm must repeat the namespace"),
            "{body}"
        );
    }
    assert_eq!(namespace_rows(&store, &ns).await, before);

    // A running job of the namespace refuses the clear, an expired lease does not.
    let request = RetainRequest {
        namespace: busy.clone(),
        session_id: "session".into(),
        external_id: Uuid::new_v4().to_string(),
        metadata: json!({}),
        events: vec![EvidenceEvent {
            role: "user".into(),
            content: "Ada uses Rust".into(),
            occurred_at: at,
            metadata: json!({}),
        }],
    };
    let ready = READY_EXTRACT_JOBS.read().await;
    let (_, job, _) = store.retain(&request).await.unwrap();
    drop(ready);
    sqlx::query("UPDATE memory_jobs SET status='running',lease_token=gen_random_uuid(),lease_until=now()+interval '15 minutes' WHERE id=$1").bind(job).execute(&store.pool).await.unwrap();
    let (status, body) = post_json(
        &app,
        "/api/v2/graph/clear",
        &json!({"namespace":busy,"confirm":busy}),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(namespace_rows(&store, &busy).await["episodes"], 1);
    sqlx::query("UPDATE memory_jobs SET lease_until=now()-interval '1 minute' WHERE id=$1")
        .bind(job)
        .execute(&store.pool)
        .await
        .unwrap();

    // The clear removes the whole namespace, in PostgreSQL and in Neo4j.
    let (status, body) = post_json(
        &app,
        "/api/v2/graph/clear",
        &json!({"namespace":ns,"confirm":ns}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"]["assertions"], 2, "{body}");
    assert_eq!(body["deleted"]["episodes"], 2, "{body}");
    assert_eq!(body["deleted"]["events"], 2, "{body}");
    assert_eq!(body["deleted"]["chunks"], 2, "{body}");
    assert_eq!(body["deleted"]["sessions"], 1, "{body}");
    assert_eq!(body["deleted"]["legacy_memories"], 1, "{body}");
    assert_eq!(body["graph"]["cleared"], true, "{body}");
    let after = namespace_rows(&store, &ns).await;
    for table in [
        "assertions",
        "entities",
        "episodes",
        "episode_sources",
        "assertion_edges",
        "events",
        "chunks",
        "sessions",
        "work_jobs",
        "legacy_memories",
        "legacy_sources",
    ] {
        assert_eq!(after[table], 0, "{table} of the cleared namespace: {after}");
    }
    let (traces, linked): (i64, i64) = sqlx::query_as("SELECT count(*),count(*) FILTER(WHERE session_id IS NOT NULL OR event_id IS NOT NULL) FROM operations WHERE namespace=$1 AND operation_type='ingest'")
        .bind(&ns)
        .fetch_one(&store.pool)
        .await
        .unwrap();
    assert_eq!(
        (traces, linked),
        (1, 0),
        "the trace stays, without its links to rows that are gone"
    );
    assert_eq!(
        after["clear_jobs"], 1,
        "the Neo4j clear job stays queued as the retry: {after}"
    );
    assert_eq!(neo4j_nodes(&graph, &ns).await, 0);

    // Nothing of the other namespace changed, and clearing again is harmless.
    assert_eq!(namespace_rows(&store, &other).await, other_before);
    assert_eq!(neo4j_nodes(&graph, &other).await, other_nodes);
    let (status, body) = post_json(
        &app,
        "/api/v2/graph/clear",
        &json!({"namespace":ns,"confirm":ns}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["deleted"]["assertions"], 0, "{body}");

    // A fact retained and projected after the clear is not dropped by the clear's revision gate.
    extract(
        &store,
        &ns,
        "Ada uses Zig",
        at,
        vec![claim("Zig", Cardinality::Single, false)],
    )
    .await;
    project_namespace(&store, &graph, &ns).await;
    assert!(
        neo4j_nodes(&graph, &ns).await >= 2,
        "new facts are projected after a clear"
    );

    // The busy namespace clears once its lease expired.
    let (status, body) = post_json(
        &app,
        "/api/v2/graph/clear",
        &json!({"namespace":busy,"confirm":busy}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(namespace_rows(&store, &busy).await["episodes"], 0);
}
#[tokio::test]
async fn clearing_the_graph_removes_one_namespace_completely_and_leaves_the_others() {
    let Some((store, graph)) =
        graph_view_services("clearing_the_graph_removes_one_namespace").await
    else {
        return;
    };
    let ns = format!("clear-{}", Uuid::new_v4());
    let other = format!("clear-other-{}", Uuid::new_v4());
    let busy = format!("clear-busy-{}", Uuid::new_v4());
    let outcome = tokio::spawn(clear_checks(
        store.clone(),
        graph.clone(),
        ns.clone(),
        other.clone(),
        busy.clone(),
    ))
    .await;
    // Remove what the test left. The jobs go first and by plain SQL: a pending extract job left behind would be claimed by
    // the claim test, which takes any ready extract job in the shared database, so a failure here must never leave one.
    let all = vec![ns.clone(), other.clone(), busy.clone()];
    sqlx::query("DELETE FROM memory_jobs WHERE namespace = ANY($1)")
        .bind(&all)
        .execute(&store.pool)
        .await
        .unwrap();
    // The rest is removed with the function under test, so the shared databases do not grow. Every namespace is tried
    // before a failure is reported.
    let mut cleanup_errors = Vec::new();
    for namespace in [&ns, &other, &busy] {
        if let Err(error) = store.clear_namespace(namespace).await {
            cleanup_errors.push(format!("{namespace}: {error}"));
        }
        delete_projected_namespace(&graph, namespace).await;
    }
    sqlx::query("DELETE FROM memory_jobs WHERE namespace = ANY($1)")
        .bind(&all)
        .execute(&store.pool)
        .await
        .unwrap();
    if let Err(error) = outcome {
        match error.try_into_panic() {
            Ok(panic) => std::panic::resume_unwind(panic),
            Err(error) => panic!("clear checks did not finish: {error}"),
        }
    }
    assert!(
        cleanup_errors.is_empty(),
        "cleanup with clear_namespace failed: {cleanup_errors:?}"
    );
}

#[tokio::test]
async fn frontend_assets_are_served_and_retired_routes_are_gone() {
    let app = api::router(view_state(offline_store(), None));
    for (path, content_type, contains) in [
        ("/", "text/html", "id=\"root\""),
        ("/graph?namespace=review", "text/html", "id=\"root\""),
        ("/assets/app.js", "text/javascript", "Memory workspace"),
        ("/assets/app.css", "text/css", "graph-mode"),
    ] {
        let response = app
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "{path}");
        assert!(
            response.headers()["content-type"]
                .to_str()
                .unwrap()
                .starts_with(content_type)
        );
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            std::str::from_utf8(&bytes).unwrap().contains(contains),
            "{path}"
        );
    }
    for (method, path) in [
        ("POST", "/api/v1/events"),
        ("POST", "/api/v1/sessions"),
        ("POST", "/api/v1/search"),
        ("POST", "/api/v1/demo/reset"),
        ("GET", "/api/v1/memories"),
        ("GET", "/api/v1/traces"),
        ("GET", "/assets/missing.js"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn metrics_report_durable_operations_instead_of_unused_counters() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        return;
    };
    let store = Store::new(PgPoolOptions::new().connect(&url).await.unwrap());
    store.migrate().await.unwrap();
    let operation = "reflect_v2";
    let before: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM operations WHERE operation_type=$1 AND status='succeeded'",
    )
    .bind(operation)
    .fetch_one(&store.pool)
    .await
    .unwrap();
    let trace =
        memory_engine::observability::TraceBuilder::new(operation, "metrics-test", None, json!({}))
            .finish("succeeded", false, None, None);
    store.record_trace(&trace).await.unwrap();
    let app = api::router(view_state(store.clone(), None));
    let response = app
        .oneshot(Request::get("/metrics").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    let output = std::str::from_utf8(&bytes).unwrap();
    assert!(output.contains(&format!(
        "memory_engine_operations_total{{operation=\"{operation}\",status=\"succeeded\"}} {}",
        before + 1
    )));
    assert!(!output.contains("memory_engine_versions_created_total"));
    sqlx::query("DELETE FROM operations WHERE id=$1")
        .bind(trace.id)
        .execute(&store.pool)
        .await
        .unwrap();
}
