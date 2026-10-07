use crate::AppError;
use async_trait::async_trait;
use reqwest::Client;
use serde::Deserialize;
use serde_json::json;
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};

#[async_trait]
pub trait Embedder: Send + Sync {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, AppError>;
    async fn ready(&self) -> bool;
    fn version(&self) -> &str;
}
pub type DynEmbedder = Arc<dyn Embedder>;

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
