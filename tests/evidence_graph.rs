use async_trait::async_trait;
use chrono::{Duration, Utc};
use memory_engine::{
    AppError, AppState,
    knowledge::*,
    model::JsonModel,
    providers::{Embedder, Extractor},
    store::Store,
    v2::{RecallRequest, recall_engine},
};
use serde_json::{Value, json};
use sqlx::{Row, postgres::PgPoolOptions};
use std::sync::Arc;
use uuid::Uuid;
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
#[async_trait]
impl Extractor for NoProviders {
    async fn extract(
        &self,
        _: &str,
    ) -> Result<Vec<memory_engine::domain::ExtractedMemory>, AppError> {
        Ok(vec![])
    }
    async fn ready(&self) -> bool {
        true
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
    let (episode, job, _) = store.retain(&request).await.unwrap();
    // Claim only this fixture's job; integration tests may share a database without resets.
    sqlx::query("UPDATE memory_jobs SET status='running',attempts=1,lease_token=gen_random_uuid(),lease_until=now()+interval '15 minutes' WHERE id=$1").bind(job).execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO memory_namespace_leases SELECT namespace,id,lease_token,lease_until FROM memory_jobs WHERE id=$1 ON CONFLICT(namespace) DO UPDATE SET job_id=excluded.job_id,lease_token=excluded.lease_token,lease_until=excluded.lease_until").bind(job).execute(&store.pool).await.unwrap();
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
    let first = store.retain(&req).await.unwrap();
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
        extractor: Arc::new(NoProviders),
        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
        demo_mode: false,
        metrics: Arc::new(Default::default()),
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
        extractor: Arc::new(NoProviders),
        embedder: Arc::new(ConstantEmbedder),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
        demo_mode: false,
        metrics: Arc::new(Default::default()),
    };
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
        extractor: Arc::new(NoProviders),
        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
        demo_mode: false,
        metrics: Arc::new(Default::default()),
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
        extractor: Arc::new(NoProviders),
        embedder: Arc::new(NoProviders),
        model: Arc::new(NoModel),
        worker_concurrency: 1,
        demo_mode: false,
        metrics: Arc::new(Default::default()),
    };
    // Extraction finished and produced no assertion, so only the raw fallback can answer.
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
