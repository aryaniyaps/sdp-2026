use crate::{AppError, domain::ExtractedMemory, model::OllamaLimits};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

#[async_trait]
pub trait Extractor: Send + Sync {
    async fn extract(&self, text: &str) -> Result<Vec<ExtractedMemory>, AppError>;
    async fn ready(&self) -> bool;
    fn version(&self) -> &str;
}
#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, AppError>;
    async fn ready(&self) -> bool;
    fn version(&self) -> &str;
}
pub type DynExtractor = Arc<dyn Extractor>;
pub type DynEmbedder = Arc<dyn Embedder>;

#[derive(Clone)]
pub struct OllamaExtractor {
    client: Client,
    base: String,
    model: String,
    limits: OllamaLimits,
}
/// Recent embeddings of short texts, so asking the same question twice skips the embedder. Only texts of at most
/// `CACHE_MAX_CHARS` characters are kept, which leaves the long chunks of ingestion out, and the oldest entry goes
/// first once `CACHE_ENTRIES` are held.
#[derive(Default)]
struct QueryCache {
    vectors: HashMap<String, Vec<f32>>,
    order: VecDeque<String>,
}
const CACHE_ENTRIES: usize = 256;
const CACHE_MAX_CHARS: usize = 512;
/// How long Ollama keeps the embedding model loaded after a request. A model that was unloaded costs seconds to load again.
pub const DEFAULT_KEEP_ALIVE: &str = "1h";

#[derive(Clone)]
pub struct OllamaEmbedder {
    client: Client,
    base: String,
    model: String,
    keep_alive: String,
    cache: Arc<Mutex<QueryCache>>,
}

impl OllamaExtractor {
    pub fn new(base: String, model: String, limits: OllamaLimits) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
            base,
            model,
            limits,
        }
    }
}
impl OllamaEmbedder {
    pub fn new(base: String, model: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
            base,
            model,
            keep_alive: DEFAULT_KEEP_ALIVE.into(),
            cache: Arc::default(),
        }
    }
    /// Sets how long Ollama keeps the model loaded (a duration such as `10m` or `1h`, or `-1` for ever).
    pub fn with_keep_alive(mut self, keep_alive: String) -> Self {
        self.keep_alive = keep_alive;
        self
    }
    async fn request_embedding(&self, text: &str) -> Result<Vec<f32>, AppError> {
        let response = self
            .client
            .post(format!("{}/api/embed", self.base))
            .json(&json!({"model":self.model,"input":text,"dimensions":1024,"keep_alive":self.keep_alive}))
            .send()
            .await?
            .error_for_status()?
            .json::<EmbedResponse>()
            .await?;
        let vector = response
            .embeddings
            .into_iter()
            .next()
            .ok_or_else(|| AppError::Provider("embedder returned no vector".into()))?;
        if vector.len() != 1024 {
            return Err(AppError::Provider(format!(
                "expected 1024 embedding dimensions, got {}",
                vector.len()
            )));
        }
        Ok(vector)
    }
}

#[derive(Serialize)]
struct GenerateRequest<'a> {
    model: &'a str,
    prompt: String,
    stream: bool,
    format: serde_json::Value,
    options: serde_json::Value,
}
#[derive(Deserialize)]
struct GenerateResponse {
    response: String,
}
#[derive(Deserialize)]
struct ExtractionEnvelope {
    memories: Vec<ExtractedMemory>,
}

fn extraction_schema() -> serde_json::Value {
    json!({"type":"object","properties":{"memories":{"type":"array","items":{"type":"object","required":["subject","predicate","value","statement","kind"],"properties":{"subject":{"type":"string"},"predicate":{"type":"string"},"value":{"type":"string"},"statement":{"type":"string"},"kind":{"type":"string","enum":["preference","fact","profile","goal","other"]}}}}},"required":["memories"]})
}

#[async_trait]
impl Extractor for OllamaExtractor {
    async fn extract(&self, text: &str) -> Result<Vec<ExtractedMemory>, AppError> {
        let prompt = format!(
            "Extract durable, explicit user memories. Do not infer. Return no memory for transient requests. Text:\n{text}"
        );
        let mut last = String::new();
        for repair in 0..=1 {
            let prompt = if repair == 0 {
                prompt.clone()
            } else {
                format!(
                    "Repair this invalid response to exactly match the schema. Invalid response:\n{last}"
                )
            };
            self.limits.preflight(&prompt)?;
            let body = GenerateRequest {
                model: &self.model,
                prompt,
                stream: false,
                format: extraction_schema(),
                options: self.limits.options(Some(0)),
            };
            let response = self
                .client
                .post(format!("{}/api/generate", self.base))
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json::<serde_json::Value>()
                .await?;
            self.limits.check_response(&response)?;
            let response = serde_json::from_value::<GenerateResponse>(response)
                .map_err(|e| AppError::Provider(format!("Ollama response absent: {e}")))?;
            last = response.response;
            if let Ok(parsed) = serde_json::from_str::<ExtractionEnvelope>(&last) {
                validate_memories(&parsed.memories)?;
                return Ok(parsed.memories);
            }
        }
        Err(AppError::Provider(
            "extractor returned invalid schema after repair".into(),
        ))
    }
    async fn ready(&self) -> bool {
        self.client.post(format!("{}/api/generate", self.base)).json(&json!({"model":self.model,"prompt":"ready","stream":false,"keep_alive":"10m","options":{"num_ctx":self.limits.num_ctx,"num_predict":1}})).send().await.map(|r| r.status().is_success()).unwrap_or(false)
    }
    fn version(&self) -> &str {
        &self.model
    }
}

fn validate_memories(items: &[ExtractedMemory]) -> Result<(), AppError> {
    for m in items {
        if [&m.subject, &m.predicate, &m.value, &m.statement]
            .iter()
            .any(|s| s.trim().is_empty())
        {
            return Err(AppError::Provider(
                "extractor returned empty required field".into(),
            ));
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct EmbedResponse {
    embeddings: Vec<Vec<f32>>,
}
#[async_trait]
impl Embedder for OllamaEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, AppError> {
        let cacheable = text.chars().count() <= CACHE_MAX_CHARS;
        if cacheable
            && let Some(vector) = self
                .cache
                .lock()
                .expect("embedding cache lock")
                .vectors
                .get(text)
        {
            return Ok(vector.clone());
        }
        let vector = self.request_embedding(text).await?;
        if cacheable {
            let mut cache = self.cache.lock().expect("embedding cache lock");
            if cache
                .vectors
                .insert(text.to_string(), vector.clone())
                .is_none()
            {
                cache.order.push_back(text.to_string());
                if cache.order.len() > CACHE_ENTRIES
                    && let Some(oldest) = cache.order.pop_front()
                {
                    cache.vectors.remove(&oldest);
                }
            }
        }
        Ok(vector)
    }
    /// Asks Ollama itself, never the cache, so readiness cannot report a model that has gone away.
    async fn ready(&self) -> bool {
        self.request_embedding("ready").await.is_ok()
    }
    fn version(&self) -> &str {
        &self.model
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_empty_fields() {
        let bad = ExtractedMemory {
            subject: "".into(),
            predicate: "p".into(),
            value: "v".into(),
            statement: "s".into(),
            kind: crate::domain::MemoryKind::Fact,
        };
        assert!(validate_memories(&[bad]).is_err());
    }
    #[test]
    fn malformed_response_is_rejected() {
        assert!(serde_json::from_str::<ExtractionEnvelope>("not json").is_err());
    }
}
