use crate::{AppError, domain::ExtractedMemory};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{sync::Arc, time::Duration};

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
}
#[derive(Clone)]
pub struct OllamaEmbedder {
    client: Client,
    base: String,
    model: String,
}

impl OllamaExtractor {
    pub fn new(base: String, model: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
            base,
            model,
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
        }
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
            let body = GenerateRequest {
                model: &self.model,
                prompt,
                stream: false,
                format: extraction_schema(),
                options: json!({"temperature":0}),
            };
            let response = self
                .client
                .post(format!("{}/api/generate", self.base))
                .json(&body)
                .send()
                .await?
                .error_for_status()?
                .json::<GenerateResponse>()
                .await?;
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
        self.client.post(format!("{}/api/generate", self.base)).json(&json!({"model":self.model,"prompt":"ready","stream":false,"keep_alive":"10m","options":{"num_predict":1}})).send().await.map(|r| r.status().is_success()).unwrap_or(false)
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
        let response = self
            .client
            .post(format!("{}/api/embed", self.base))
            .json(&json!({"model":self.model,"input":text,"dimensions":1024,"keep_alive":"10m"}))
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
    async fn ready(&self) -> bool {
        self.embed("ready").await.is_ok()
    }
    fn version(&self) -> &str {
        &self.model
    }
}

// ---------------------------------------------------------------------------
// Azure OpenAI providers.
//
// The Ollama providers require a local model server, which rules out hosting on
// CPU-only container platforms: the 14B extractor needs a GPU and the embedder
// needs a resident model. These implement the same two traits against Azure
// OpenAI, so the storage design and the rest of the pipeline are untouched.
//
// Embeddings use text-embedding-3-small at dimensions=1024 to match the
// vector(1024) column in migrations/0001. The text-embedding-3 family is trained
// with Matryoshka representation learning, so a 1024-d vector is a valid
// embedding rather than a truncated one.

#[derive(Clone)]
pub struct AzureEmbedder {
    client: Client,
    endpoint: String,
    api_key: String,
    deployment: String,
    api_version: String,
}

#[derive(Clone)]
pub struct AzureExtractor {
    client: Client,
    endpoint: String,
    api_key: String,
    deployment: String,
    api_version: String,
}

impl AzureEmbedder {
    pub fn new(endpoint: String, api_key: String, deployment: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap(),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            api_key,
            deployment,
            api_version: "2024-10-21".into(),
        }
    }
}

impl AzureExtractor {
    pub fn new(endpoint: String, api_key: String, deployment: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(120))
                .build()
                .unwrap(),
            endpoint: endpoint.trim_end_matches('/').to_string(),
            api_key,
            deployment,
            api_version: "2025-04-01-preview".into(),
        }
    }
}

#[derive(Deserialize)]
struct AzureEmbedItem {
    embedding: Vec<f32>,
}
#[derive(Deserialize)]
struct AzureEmbedResponse {
    data: Vec<AzureEmbedItem>,
}

#[async_trait]
impl Embedder for AzureEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, AppError> {
        let url = format!(
            "{}/openai/deployments/{}/embeddings?api-version={}",
            self.endpoint, self.deployment, self.api_version
        );
        let response = self
            .client
            .post(url)
            .header("api-key", &self.api_key)
            .json(&json!({"input": text, "dimensions": 1024}))
            .send()
            .await?
            .error_for_status()?
            .json::<AzureEmbedResponse>()
            .await?;
        let vector = response
            .data
            .into_iter()
            .next()
            .map(|d| d.embedding)
            .ok_or_else(|| AppError::Provider("embedder returned no vector".into()))?;
        if vector.len() != 1024 {
            return Err(AppError::Provider(format!(
                "expected 1024 embedding dimensions, got {}",
                vector.len()
            )));
        }
        Ok(vector)
    }
    async fn ready(&self) -> bool {
        self.embed("ready").await.is_ok()
    }
    fn version(&self) -> &str {
        &self.deployment
    }
}

#[derive(Deserialize)]
struct AzureChatMessage {
    content: Option<String>,
}
#[derive(Deserialize)]
struct AzureChatChoice {
    message: AzureChatMessage,
}
#[derive(Deserialize)]
struct AzureChatResponse {
    choices: Vec<AzureChatChoice>,
}

#[async_trait]
impl Extractor for AzureExtractor {
    async fn extract(&self, text: &str) -> Result<Vec<ExtractedMemory>, AppError> {
        let url = format!(
            "{}/openai/deployments/{}/chat/completions?api-version={}",
            self.endpoint, self.deployment, self.api_version
        );
        // strict json_schema makes the structured-output guarantee the model's
        // problem rather than ours, so no repair round-trip is needed.
        let schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["memories"],
            "properties": {"memories": {"type": "array", "items": {
                "type": "object",
                "additionalProperties": false,
                "required": ["subject", "predicate", "value", "statement", "kind"],
                "properties": {
                    "subject": {"type": "string"},
                    "predicate": {"type": "string"},
                    "value": {"type": "string"},
                    "statement": {"type": "string"},
                    "kind": {"type": "string",
                             "enum": ["preference", "fact", "profile", "goal", "other"]}
                }}}}
        });
        let body = json!({
            "messages": [
                {"role": "system", "content":
                 "Extract durable, explicit user memories. Do not infer. Return no \
                  memory for transient requests, greetings or small talk. One atomic \
                  claim per memory.\n\n\
                  The predicate is a canonical identity, not a description. Two \
                  statements about the same relation must produce the same predicate \
                  or the store cannot tell that one replaces the other. So use the \
                  simplest present-tense verb for the relation and put every specific \
                  detail in the value:\n\
                  - a change of car is predicate `drives`, not `traded_vehicle_for`\n\
                  - a change of home is `lives_in`, not `relocated_to`\n\
                  - a change of job is `works_at`, not `started_new_role_at`\n\
                  Prefer: drives, owns, lives_in, works_at, studies, prefers, likes, \
                  dislikes, plays, reads, uses, plans_to, is. Never encode tense, \
                  change or manner in the predicate."},
                {"role": "user", "content": format!("Text:\n{text}")}
            ],
            "response_format": {"type": "json_schema", "json_schema":
                {"name": "extraction", "strict": true, "schema": schema}},
        });
        let response = self
            .client
            .post(url)
            .header("api-key", &self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json::<AzureChatResponse>()
            .await?;
        let raw = response
            .choices
            .into_iter()
            .next()
            .and_then(|c| c.message.content)
            .ok_or_else(|| AppError::Provider("extractor returned no content".into()))?;
        let parsed = serde_json::from_str::<ExtractionEnvelope>(&raw)
            .map_err(|e| AppError::Provider(format!("extractor returned invalid schema: {e}")))?;
        validate_memories(&parsed.memories)?;
        Ok(parsed.memories)
    }
    async fn ready(&self) -> bool {
        !self.api_key.is_empty() && !self.endpoint.is_empty()
    }
    fn version(&self) -> &str {
        &self.deployment
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
