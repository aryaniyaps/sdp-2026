//! Local Ollama context handling: every generate call carries num_ctx and num_predict, a prompt
//! that cannot fit is refused before any request, and an answer to a truncated prompt is never
//! accepted or cached. The fake server stands in for Ollama and records what it receives.
//!
//! The `live_*` tests talk to a real Ollama. They run only when OLLAMA_LIVE_TEST=1 is set, and
//! then fail loudly if Ollama or its model is unreachable. OLLAMA_URL and EXTRACTION_MODEL
//! override 127.0.0.1:11434 and qwen2.5:14b-instruct-q4_K_M.
use async_trait::async_trait;
use axum::{
    Json, Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    routing::post,
};
use http_body_util::BodyExt;
use memory_engine::{
    AppError, AppState, api,
    knowledge::{EvidenceEvent, Job, RetainRequest},
    model::{JsonModel, OllamaJsonModel, OllamaLimits, cached_generate, estimate_tokens},
    providers::Embedder,
    store::Store,
};
use serde_json::{Value, json};
use sqlx::postgres::PgPoolOptions;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;
use uuid::Uuid;

struct FakeOllama {
    requests: Mutex<Vec<Value>>,
    reply: Mutex<Value>,
}
impl FakeOllama {
    fn requests(&self) -> Vec<Value> {
        self.requests.lock().unwrap().clone()
    }
    fn reply_with(&self, reply: Value) {
        *self.reply.lock().unwrap() = reply;
    }
}
async fn generate(State(fake): State<Arc<FakeOllama>>, Json(body): Json<Value>) -> Json<Value> {
    fake.requests.lock().unwrap().push(body);
    Json(fake.reply.lock().unwrap().clone())
}
fn normal_reply(prompt_eval_count: usize) -> Value {
    json!({"response":"{\"ok\":true}","done":true,"done_reason":"stop","prompt_eval_count":prompt_eval_count,"eval_count":7})
}
async fn fake_ollama(reply: Value) -> (String, Arc<FakeOllama>) {
    let fake = Arc::new(FakeOllama {
        requests: Mutex::new(Vec::new()),
        reply: Mutex::new(reply),
    });
    let app = Router::new()
        .route("/api/generate", post(generate))
        .with_state(fake.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (base, fake)
}
fn limits() -> OllamaLimits {
    OllamaLimits::new(16384, 4096).unwrap()
}
fn model(base: &str) -> OllamaJsonModel {
    OllamaJsonModel {
        base: base.into(),
        model: "fake-model".into(),
        limits: limits(),
    }
}

#[tokio::test]
async fn every_generate_call_carries_context_and_output_cap() {
    let (base, fake) = fake_ollama(normal_reply(120)).await;
    let value = model(&base).generate("short prompt").await.unwrap();
    assert_eq!(value, json!({"ok":true}));
    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["options"]["num_ctx"], 16384);
    assert_eq!(requests[0]["options"]["num_predict"], 4096);
    assert_eq!(requests[0]["options"]["temperature"], 0);
    assert_eq!(requests[0]["format"], "json");
    assert_eq!(requests[0]["stream"], false);
    assert_eq!(requests[0]["prompt"], "short prompt");
}

#[tokio::test]
async fn prompt_that_cannot_fit_fails_before_any_request() {
    let (base, fake) = fake_ollama(normal_reply(120)).await;
    // 12288 tokens are available for the prompt. 33510 letters are estimated at 12287 tokens,
    // one more letter at 12289.
    let fits = "x".repeat(33510);
    assert_eq!(estimate_tokens(&fits), 12287);
    model(&base).generate(&fits).await.unwrap();
    let too_big = "x".repeat(33511);
    let error = model(&base)
        .generate(&too_big)
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("12289 tokens"), "{error}");
    assert!(error.contains("16384"), "{error}");
    assert!(error.contains("OLLAMA_NUM_CTX"), "{error}");
    assert_eq!(fake.requests().len(), 1, "only the fitting prompt was sent");
}

#[tokio::test]
async fn prompt_evaluated_up_to_the_context_is_rejected() {
    let (base, fake) = fake_ollama(normal_reply(16384)).await;
    let error = model(&base)
        .generate("looks small but the server says otherwise")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("prompt was truncated by Ollama"), "{error}");
    assert_eq!(fake.requests().len(), 1);
}

#[tokio::test]
async fn answer_cut_at_the_output_cap_is_rejected() {
    let (base, _) = fake_ollama(
        json!({"response":"{\"events\":[","done":true,"done_reason":"length","prompt_eval_count":500,"eval_count":4096}),
    )
    .await;
    let error = model(&base).generate("p").await.unwrap_err().to_string();
    assert!(error.contains("OLLAMA_NUM_PREDICT"), "{error}");
}

#[tokio::test]
async fn truncated_answer_is_not_cached_and_a_normal_one_is() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; cache test skipped");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let store = Store::new(pool);
    store.migrate().await.unwrap();
    let (base, fake) = fake_ollama(normal_reply(16384)).await;
    let model = model(&base);
    let version = format!("ctx-test-{}", Uuid::new_v4());
    let cached = |store: &Store| {
        let version = version.clone();
        let store = store.clone();
        async move {
            sqlx::query_scalar::<_, i64>("SELECT count(*) FROM model_cache WHERE prompt_version=$1")
                .bind(version)
                .fetch_one(&store.pool)
                .await
                .unwrap()
        }
    };
    let error = cached_generate(&store, &model, &version, "same prompt")
        .await
        .unwrap_err();
    assert!(error.to_string().contains("truncated"), "{error}");
    assert_eq!(
        cached(&store).await,
        0,
        "a truncated answer must not be cached"
    );

    fake.reply_with(normal_reply(900));
    let first = cached_generate(&store, &model, &version, "same prompt")
        .await
        .unwrap();
    assert_eq!(first, json!({"ok":true}));
    assert_eq!(
        fake.requests().len(),
        2,
        "the rejected answer was not replayed"
    );
    assert_eq!(cached(&store).await, 1);
    let second = cached_generate(&store, &model, &version, "same prompt")
        .await
        .unwrap();
    assert_eq!(second, first);
    assert_eq!(
        fake.requests().len(),
        2,
        "the normal answer came from the cache"
    );
    sqlx::query("DELETE FROM model_cache WHERE prompt_version=$1")
        .bind(&version)
        .execute(&store.pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn cache_is_keyed_on_the_context_size() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; cache test skipped");
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
    let store = Store::new(pool);
    store.migrate().await.unwrap();
    let (base, fake) = fake_ollama(normal_reply(900)).await;
    let version = format!("ctx-key-{}", Uuid::new_v4());
    let small = OllamaJsonModel {
        limits: OllamaLimits::new(4096, 1024).unwrap(),
        ..model(&base)
    };
    cached_generate(&store, &small, &version, "p")
        .await
        .unwrap();
    cached_generate(&store, &model(&base), &version, "p")
        .await
        .unwrap();
    assert_eq!(
        fake.requests().len(),
        2,
        "an answer made under another context is not replayed"
    );
    sqlx::query("DELETE FROM model_cache WHERE prompt_version=$1")
        .bind(&version)
        .execute(&store.pool)
        .await
        .unwrap();
}

struct Unused;
#[async_trait]
impl Embedder for Unused {
    async fn embed(&self, _: &str) -> Result<Vec<f32>, AppError> {
        Err(AppError::Provider("unused".into()))
    }
    async fn ready(&self) -> bool {
        true
    }
    fn version(&self) -> &str {
        "embed-test"
    }
}

#[tokio::test]
async fn healthz_reports_the_local_model_and_its_context() {
    let (base, _) = fake_ollama(normal_reply(1)).await;
    // The health check only needs the database to be reported as down, quickly.
    let pool = PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect_lazy("postgres://nobody@127.0.0.1:1/none")
        .unwrap();
    let state = |model: Arc<dyn JsonModel>| {
        Arc::new(AppState {
            store: Store::new(pool.clone()),
            graph: None,
            embedder: Arc::new(Unused),
            model,

            worker_concurrency: 1,
        })
    };
    let health = |state: Arc<AppState>| async move {
        let response = api::router(state)
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice::<Value>(&bytes).unwrap()
    };
    let local = health(state(Arc::new(model(&base)))).await;
    assert_eq!(local["worker_model"], "ollama/fake-model");
    assert_eq!(local["worker_num_ctx"], 16384);
    assert_eq!(local["worker_num_predict"], 4096);

    let hosted = health(state(Arc::new(memory_engine::model::PiModel {
        executable: "pi".into(),
        provider: "p".into(),
        model: "m".into(),
    })))
    .await;
    assert_eq!(hosted["worker_model"], "pi/p/m");
    assert!(hosted["worker_num_ctx"].is_null());
}

// Opt in tests against a real Ollama.

fn live_base() -> Option<(String, String)> {
    if std::env::var("OLLAMA_LIVE_TEST").as_deref() != Ok("1") {
        eprintln!("OLLAMA_LIVE_TEST is not 1; live Ollama test not run");
        return None;
    }
    Some((
        std::env::var("OLLAMA_URL").unwrap_or_else(|_| "http://127.0.0.1:11434".into()),
        std::env::var("EXTRACTION_MODEL").unwrap_or_else(|_| "qwen2.5:14b-instruct-q4_K_M".into()),
    ))
}
/// Repository text, repeated and cut to a character count.
fn repository_text(chars: usize) -> String {
    let source = [
        include_str!("../src/v2/recall.rs"),
        include_str!("../src/worker/mod.rs"),
        include_str!("../src/graph.rs"),
        include_str!("../docs/evidence-memory.md"),
        include_str!("../README.md"),
    ]
    .join("\n");
    source.chars().cycle().take(chars).collect()
}
async fn live_generate(base: &str, model: &str, prompt: &str) -> Value {
    let response = reqwest::Client::new()
        .post(format!("{base}/api/generate"))
        .timeout(std::time::Duration::from_secs(600))
        .json(&json!({"model":model,"prompt":prompt,"stream":false,"options":limits().options(Some(0))}))
        .send()
        .await
        .unwrap_or_else(|e| panic!("live Ollama at {base} is unreachable: {e}"));
    assert!(
        response.status().is_success(),
        "live Ollama answered {}",
        response.status()
    );
    response.json().await.unwrap()
}

#[tokio::test]
async fn live_prompt_of_about_nine_thousand_tokens_is_not_cut_to_4096() {
    let Some((base, model)) = live_base() else {
        return;
    };
    let prompt = format!(
        "{}\n\nReply with the single word OK.",
        repository_text(36000)
    );
    let response = live_generate(&base, &model, &prompt).await;
    let evaluated = response["prompt_eval_count"].as_u64().unwrap() as usize;
    eprintln!(
        "live: {} chars, estimate {} tokens, prompt_eval_count {evaluated}",
        prompt.chars().count(),
        estimate_tokens(&prompt)
    );
    assert!(evaluated > 4096, "Ollama still used a 4096 token window");
    assert!(
        evaluated <= estimate_tokens(&prompt),
        "the chars/3 estimate must not undercount: real {evaluated}"
    );
    limits().check_response(&response).unwrap();
}

#[tokio::test]
async fn live_prompt_beyond_the_context_is_rejected_by_the_service_code_path() {
    let Some((base, model)) = live_base() else {
        return;
    };
    let prompt = format!(
        "{}\n\nReply with the single word OK.",
        repository_text(90000)
    );
    let local = OllamaJsonModel {
        base: base.clone(),
        model: model.clone(),
        limits: limits(),
    };
    // The service refuses before any request.
    let error = local.generate(&prompt).await.unwrap_err().to_string();
    assert!(error.contains("OLLAMA_NUM_CTX"), "{error}");
    // Without the preflight Ollama would accept it silently and cut it; the response check
    // catches that.
    let response = live_generate(&base, &model, &prompt).await;
    eprintln!(
        "live: estimate {} tokens, prompt_eval_count {}",
        estimate_tokens(&prompt),
        response["prompt_eval_count"]
    );
    let error = limits().check_response(&response).unwrap_err().to_string();
    assert!(error.contains("prompt was truncated by Ollama"), "{error}");
}

#[tokio::test]
async fn live_shrunk_extract_prompt_fits_the_real_context() {
    let Some((base, model)) = live_base() else {
        return;
    };
    let facts: Vec<Value> = (0..80)
        .map(|n| {
            json!({"id":Uuid::new_v4(),"subject":format!("Person {n}"),"subject_id":Uuid::new_v4(),"predicate":"likes","value":"hiking in the mountains","cardinality":"multiple","valid_from":"2026-01-02T03:04:05.123456Z"})
        })
        .collect();
    let events = vec![
        json!({"source_index":0,"event":{"role":"user","content":repository_text(20000),"occurred_at":"2026-02-03T04:05:06Z","metadata":null}}),
    ];
    let full = memory_engine::worker::extract_prompt(&facts, &events);
    assert!(
        limits().preflight(&full).is_err(),
        "the unshrunk prompt must not fit, or this test proves nothing"
    );
    let (kept, dropped) = memory_engine::worker::fit_existing_facts(&facts, &events, &limits());
    let prompt = format!(
        "{}\n\nIgnore the task above and reply with the single word OK.",
        memory_engine::worker::extract_prompt(&kept, &events)
    );
    limits().preflight(&prompt).unwrap();
    let response = live_generate(&base, &model, &prompt).await;
    let evaluated = response["prompt_eval_count"].as_u64().unwrap() as usize;
    eprintln!(
        "live: kept {} dropped {dropped}, estimate {} tokens, prompt_eval_count {evaluated}",
        kept.len(),
        estimate_tokens(&prompt)
    );
    assert!(dropped > 0);
    assert!(
        evaluated <= estimate_tokens(&prompt),
        "estimate undercounted: real {evaluated}"
    );
    limits().check_response(&response).unwrap();
}

/// Records the prompts it is asked and answers with no claims.
struct Recording {
    prompts: Mutex<Vec<String>>,
    limits: Option<OllamaLimits>,
}
#[async_trait]
impl JsonModel for Recording {
    async fn generate(&self, prompt: &str) -> Result<Value, AppError> {
        self.prompts.lock().unwrap().push(prompt.into());
        Ok(json!({"claims":[]}))
    }
    fn identity(&self) -> String {
        "recording-fixture".into()
    }
    fn context_limits(&self) -> Option<OllamaLimits> {
        self.limits
    }
}
fn snapshot_facts(count: usize) -> Vec<Value> {
    (0..count)
        .map(|n| {
            json!({"id":Uuid::new_v4(),"subject":format!("Subject {n}"),"subject_id":Uuid::new_v4(),"predicate":"likes","value":format!("thing number {n} with some padding words"),"cardinality":"multiple","valid_from":"2026-01-02T00:00:00Z"})
        })
        .collect()
}
/// Retains one event, then turns its extract job into a running job owned by this test, so no
/// other job in the database can be claimed or disturbed. Returns the job and the evidence
/// events as the worker will see them.
async fn running_extract_job(
    store: &Store,
    facts: &[Value],
    content: &str,
) -> (Job, Vec<EvidenceEvent>) {
    let ns = format!("ctx-extract-{}", Uuid::new_v4());
    let (episode, job_id, _) = store
        .retain(&RetainRequest {
            namespace: ns.clone(),
            session_id: "s".into(),
            external_id: "e".into(),
            metadata: json!({}),
            events: vec![EvidenceEvent {
                role: "user".into(),
                content: format!("{content} ({ns})"),
                occurred_at: chrono::Utc::now(),
                metadata: json!({}),
            }],
        })
        .await
        .unwrap();
    let token = Uuid::new_v4();
    sqlx::query("UPDATE memory_jobs SET status='running',attempts=1,lease_token=$2,lease_until=now()+interval '15 minutes',payload=payload || jsonb_build_object('existing_snapshot',$3::jsonb) WHERE id=$1")
        .bind(job_id).bind(token).bind(json!(facts)).execute(&store.pool).await.unwrap();
    sqlx::query("INSERT INTO memory_namespace_leases(namespace,job_id,lease_token,lease_until) VALUES($1,$2,$3,now()+interval '15 minutes')")
        .bind(&ns).bind(job_id).bind(token).execute(&store.pool).await.unwrap();
    let job = Job {
        id: job_id,
        namespace: ns.clone(),
        kind: "extract".into(),
        payload: json!({"episode_id":episode,"existing_snapshot":facts}),
        status: "running".into(),
        attempts: 1,
        lease_token: Some(token),
        result: None,
        error: None,
    };
    let events = store
        .episode_evidence(&ns, episode)
        .await
        .unwrap()
        .into_iter()
        .map(|(_, event)| event)
        .collect();
    (job, events)
}
async fn extract_with(
    store: &Store,
    model: Arc<Recording>,
    facts: &[Value],
    content: &str,
) -> (Value, String, Vec<Value>, Vec<EvidenceEvent>) {
    let (job, events) = running_extract_job(store, facts, content).await;
    let state = AppState {
        store: store.clone(),
        graph: None,
        embedder: Arc::new(Unused),
        model: model.clone(),

        worker_concurrency: 1,
    };
    let result = memory_engine::worker::process(&state, &job).await.unwrap();
    store.finish_job(&job, Ok(result.clone())).await.unwrap();
    let prompts = model.prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 1);
    let event_values = events
        .iter()
        .enumerate()
        .map(|(index, event)| json!({"source_index":index,"event":event}))
        .collect();
    (result, prompts[0].clone(), event_values, events)
}

#[tokio::test]
async fn worker_shrinks_the_snapshot_for_a_local_model_and_leaves_hosted_alone() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; worker test skipped");
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
    let facts = snapshot_facts(80);

    // A small window forces the shrink; the fact the batch talks about must survive it.
    let small = OllamaLimits::new(4096, 1024).unwrap();
    let local = Arc::new(Recording {
        prompts: Mutex::new(Vec::new()),
        limits: Some(small),
    });
    let (result, prompt, _, _) =
        extract_with(&store, local, &facts, "Subject 77 now prefers tea.").await;
    let dropped = result["existing_facts_dropped"].as_u64().unwrap() as usize;
    assert!(dropped > 0 && dropped < 80, "{result}");
    assert!(
        estimate_tokens(&prompt) + small.num_predict <= small.num_ctx,
        "prompt must fit with room for the answer"
    );
    assert!(prompt.starts_with("Extract durable explicit facts"));
    assert!(prompt.contains("Existing facts: ["));
    assert!(
        prompt.contains(facts[77]["id"].as_str().unwrap()),
        "the fact the batch talks about is kept"
    );
    assert!(
        !prompt.contains(facts[79]["id"].as_str().unwrap()),
        "the oldest fact that is not mentioned goes first"
    );

    // Hosted models have no known window: every fact stays and the prompt is the old one.
    let hosted = Arc::new(Recording {
        prompts: Mutex::new(Vec::new()),
        limits: None,
    });
    let (result, prompt, event_values, _) =
        extract_with(&store, hosted, &facts, "Subject 77 now prefers tea.").await;
    assert!(result.get("existing_facts_dropped").is_none(), "{result}");
    assert_eq!(
        prompt,
        memory_engine::worker::extract_prompt(&facts, &event_values)
    );
    assert!(prompt.contains(facts[79]["id"].as_str().unwrap()));
    sqlx::query("DELETE FROM model_cache WHERE model='recording-fixture'")
        .execute(&store.pool)
        .await
        .unwrap();
}

/// Answers the first call with a reply that fails validation and refuses every later call, as
/// the preflight does for a repair prompt that no longer fits the context.
struct RefusesRepair {
    calls: Mutex<usize>,
}
#[async_trait]
impl JsonModel for RefusesRepair {
    async fn generate(&self, _prompt: &str) -> Result<Value, AppError> {
        let mut calls = self.calls.lock().unwrap();
        *calls += 1;
        if *calls == 1 {
            Ok(json!({"events":[]}))
        } else {
            Err(AppError::Provider(
                "prompt is estimated above the context; raise OLLAMA_NUM_CTX".into(),
            ))
        }
    }
    fn identity(&self) -> String {
        "refuses-repair-fixture".into()
    }
    fn context_limits(&self) -> Option<OllamaLimits> {
        None
    }
}

#[tokio::test]
async fn refused_repair_prompt_forgets_the_invalid_reply_it_followed() {
    let Ok(url) = std::env::var("TEST_DATABASE_URL") else {
        eprintln!("TEST_DATABASE_URL missing; worker test skipped");
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
    sqlx::query("DELETE FROM model_cache WHERE model='refuses-repair-fixture'")
        .execute(&store.pool)
        .await
        .unwrap();
    let (job, _) = running_extract_job(&store, &snapshot_facts(2), "Subject 1 likes tea.").await;
    let model = Arc::new(RefusesRepair {
        calls: Mutex::new(0),
    });
    let state = AppState {
        store: store.clone(),
        graph: None,
        embedder: Arc::new(Unused),
        model: model.clone(),

        worker_concurrency: 1,
    };
    let error = memory_engine::worker::process(&state, &job)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("raise OLLAMA_NUM_CTX"),
        "{error}"
    );
    assert_eq!(*model.calls.lock().unwrap(), 2);
    let cached: i64 =
        sqlx::query_scalar("SELECT count(*) FROM model_cache WHERE model='refuses-repair-fixture'")
            .fetch_one(&store.pool)
            .await
            .unwrap();
    assert_eq!(cached, 0, "the invalid first reply must not stay cached");
}
