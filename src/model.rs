//! The worker model: a small local model served by Ollama. The memory engine never calls a hosted
//! model, and never uses the coding agent's login.
use crate::{AppError, store::Store};
use async_trait::async_trait;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::time::Duration;
mod structured;

/// Extraction's wire adapter appends instructions after the canonical prompt. Budget the
/// exact text that will be sent, including on validation-repair calls.
pub(crate) fn extraction_prompt_tokens(prompt: &str) -> usize {
    estimate_tokens(&format!(
        "{prompt}\n{}",
        structured::WIRE_INSTRUCTION.trim_end()
    ))
}

#[cfg(test)]
mod wire_budget_tests {
    #[test]
    fn extraction_estimate_matches_the_preflight_wire_prompt() {
        let prompt = crate::worker::extract_prompt(&[], &[]);
        let (wire, _, _) = super::structured::prepare(&prompt).unwrap();
        assert_eq!(
            super::extraction_prompt_tokens(&prompt),
            super::estimate_tokens(&wire)
        );
    }
}

#[async_trait]
pub trait JsonModel: Send + Sync {
    async fn generate(&self, prompt: &str) -> Result<Value, AppError>;
    fn identity(&self) -> String;
    /// Context window of a local model, or `None` for hosted models that manage their own.
    fn context_limits(&self) -> Option<OllamaLimits> {
        None
    }
    /// What the model cache is keyed on. Defaults to the identity; a local model adds its
    /// context size so answers produced under another window are never replayed.
    fn cache_identity(&self) -> String {
        self.identity()
    }
}
/// Default context window requested from Ollama. Ollama itself falls back to 4096 tokens, which
/// silently truncates the extract prompt.
pub const DEFAULT_OLLAMA_NUM_CTX: usize = 16384;
/// Default cap on generated tokens, so a runaway answer cannot fill the whole context.
pub const DEFAULT_OLLAMA_NUM_PREDICT: usize = 4096;
/// A prompt whose evaluated token count comes this close to the context size was cut by Ollama.
const TRUNCATION_MARGIN_TOKENS: usize = 16;

/// Context window and output cap sent with every Ollama generate call.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OllamaLimits {
    pub num_ctx: usize,
    pub num_predict: usize,
}
/// Conservative token estimate, used before any request is made: one token per three
/// characters, except that every digit counts as a token of its own, plus ten percent. The
/// digit rule matters because the existing-facts snapshot is full of UUIDs and timestamps,
/// which Qwen tokenizes at about 1.9 characters per token; plain chars/3 undercounted that
/// part of the prompt by a third.
pub fn estimate_tokens(text: &str) -> usize {
    let digits = text.chars().filter(char::is_ascii_digit).count();
    let others = text.chars().count() - digits;
    let raw = digits + others.div_ceil(3);
    raw + raw.div_ceil(10)
}
impl OllamaLimits {
    pub fn new(num_ctx: usize, num_predict: usize) -> Result<Self, String> {
        if num_ctx == 0 {
            return Err("OLLAMA_NUM_CTX must be a positive integer".into());
        }
        if num_predict == 0 {
            return Err("OLLAMA_NUM_PREDICT must be a positive integer".into());
        }
        if num_predict >= num_ctx {
            return Err(format!(
                "OLLAMA_NUM_PREDICT ({num_predict}) must be smaller than OLLAMA_NUM_CTX ({num_ctx})"
            ));
        }
        Ok(Self {
            num_ctx,
            num_predict,
        })
    }
    /// Parse the raw values of OLLAMA_NUM_CTX and OLLAMA_NUM_PREDICT; `None` means unset.
    pub fn from_values(num_ctx: Option<&str>, num_predict: Option<&str>) -> Result<Self, String> {
        fn positive(name: &str, raw: Option<&str>, default: usize) -> Result<usize, String> {
            let Some(raw) = raw else {
                return Ok(default);
            };
            match raw.trim().parse::<usize>() {
                Ok(value) if value > 0 => Ok(value),
                _ => Err(format!("{name} must be a positive integer, got {raw:?}")),
            }
        }
        Self::new(
            positive("OLLAMA_NUM_CTX", num_ctx, DEFAULT_OLLAMA_NUM_CTX)?,
            positive(
                "OLLAMA_NUM_PREDICT",
                num_predict,
                DEFAULT_OLLAMA_NUM_PREDICT,
            )?,
        )
    }
    /// Read the two variables from the process environment. An unset variable takes its
    /// default; a set but unusable one is an error.
    pub fn from_env() -> Result<Self, String> {
        fn read(name: &str) -> Result<Option<String>, String> {
            match std::env::var(name) {
                Ok(value) => Ok(Some(value)),
                Err(std::env::VarError::NotPresent) => Ok(None),
                Err(err) => Err(format!("{name} is not usable: {err}")),
            }
        }
        Self::from_values(
            read("OLLAMA_NUM_CTX")?.as_deref(),
            read("OLLAMA_NUM_PREDICT")?.as_deref(),
        )
    }
    /// Tokens a prompt may use while leaving room for the answer.
    pub fn prompt_budget(&self) -> usize {
        self.num_ctx - self.num_predict
    }
    /// Ollama `options` for a generate call.
    pub fn options(&self, temperature: Option<u8>) -> Value {
        let mut options = json!({"num_ctx":self.num_ctx,"num_predict":self.num_predict});
        if let Some(temperature) = temperature {
            options["temperature"] = json!(temperature);
        }
        options
    }
    /// Refuse a prompt that cannot fit before any request is made.
    pub fn preflight(&self, prompt: &str) -> Result<(), AppError> {
        let estimate = estimate_tokens(prompt);
        if estimate + self.num_predict > self.num_ctx {
            return Err(AppError::Provider(format!(
                "prompt is estimated at {estimate} tokens, which with OLLAMA_NUM_PREDICT {} exceeds the Ollama context of {} tokens (OLLAMA_NUM_CTX); raise OLLAMA_NUM_CTX or shorten the prompt. No request was sent",
                self.num_predict, self.num_ctx
            )));
        }
        Ok(())
    }
    /// Reject a response whose prompt was cut or whose answer was cut off. Ollama itself
    /// reports neither as an error.
    pub fn check_response(&self, response: &Value) -> Result<(), AppError> {
        let evaluated = response["prompt_eval_count"].as_u64().ok_or_else(|| {
            AppError::Provider(
                "Ollama response has no prompt_eval_count, so truncation of the prompt cannot be ruled out".into(),
            )
        })? as usize;
        if evaluated + TRUNCATION_MARGIN_TOKENS >= self.num_ctx {
            return Err(AppError::Provider(format!(
                "prompt was truncated by Ollama: it evaluated {evaluated} prompt tokens against a context of {} (OLLAMA_NUM_CTX); raise OLLAMA_NUM_CTX or shorten the prompt",
                self.num_ctx
            )));
        }
        if response["done_reason"].as_str() == Some("length") {
            return Err(AppError::Provider(format!(
                "Ollama stopped at its token limit after {} generated tokens (OLLAMA_NUM_PREDICT {}, OLLAMA_NUM_CTX {}); the answer is incomplete",
                response["eval_count"].as_u64().unwrap_or(0),
                self.num_predict,
                self.num_ctx
            )));
        }
        Ok(())
    }
}

/// The system message of every worker call. The student is trained with this text, so it is sent
/// instead of leaving the choice to the model file.
pub const WORKER_SYSTEM: &str = include_str!("worker/worker_system.txt");

pub struct OllamaJsonModel {
    pub base: String,
    pub model: String,
    pub limits: OllamaLimits,
}
#[async_trait]
impl JsonModel for OllamaJsonModel {
    fn identity(&self) -> String {
        format!("ollama/{}", self.model)
    }
    fn context_limits(&self) -> Option<OllamaLimits> {
        Some(self.limits)
    }
    fn cache_identity(&self) -> String {
        format!(
            "{}/ctx{}/nothink/structured-v3",
            self.identity(),
            self.limits.num_ctx
        )
    }
    async fn generate(&self, prompt: &str) -> Result<Value, AppError> {
        let (wire_prompt, schema, paired) = structured::prepare(prompt)?;
        self.limits.preflight(&wire_prompt)?;
        #[derive(serde::Serialize)]
        struct Request<'a> {
            model: &'a str,
            system: &'a str,
            prompt: &'a str,
            format: &'a serde_json::value::RawValue,
            think: bool,
            stream: bool,
            keep_alive: &'a str,
            options: Value,
        }
        let body = Request {
            model: &self.model,
            system: WORKER_SYSTEM.trim_end_matches('\n'),
            prompt: &wire_prompt,
            format: &schema,
            think: false,
            stream: false,
            keep_alive: crate::providers::DEFAULT_KEEP_ALIVE,
            options: self.limits.options(Some(0)),
        };
        let response: Value = reqwest::Client::new()
            .post(format!("{}/api/generate", self.base))
            .timeout(Duration::from_secs(600))
            .json(&body)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        self.limits.check_response(&response)?;
        let value = serde_json::from_str(
            response["response"]
                .as_str()
                .ok_or_else(|| AppError::Provider("Ollama response absent".into()))?,
        )
        .map_err(|e| AppError::Provider(e.to_string()))?;
        structured::canonicalize(value, paired)
    }
}
fn cache_key(model: &dyn JsonModel, version: &str, prompt: &str) -> String {
    format!(
        "{:x}",
        Sha256::digest(format!("{}\n{version}\n{prompt}", model.cache_identity()))
    )
}
/// Drop a cached response that failed validation. Without this a retry replays
/// the same invalid answer forever and the job can never succeed.
pub async fn forget_generated(
    store: &Store,
    model: &dyn JsonModel,
    version: &str,
    prompt: &str,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM model_cache WHERE cache_key=$1")
        .bind(cache_key(model, version, prompt))
        .execute(&store.pool)
        .await?;
    Ok(())
}
pub async fn cached_generate(
    store: &Store,
    model: &dyn JsonModel,
    version: &str,
    prompt: &str,
) -> Result<Value, AppError> {
    let identity = model.identity();
    let key = cache_key(model, version, prompt);
    if let Some(v) = sqlx::query_scalar("SELECT response FROM model_cache WHERE cache_key=$1")
        .bind(&key)
        .fetch_optional(&store.pool)
        .await?
    {
        return Ok(v);
    }
    let result = model.generate(prompt).await?;
    sqlx::query("INSERT INTO model_cache(cache_key,model,prompt_version,response) VALUES($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(key).bind(identity).bind(version).bind(&result).execute(&store.pool).await?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_default_when_unset() {
        let limits = OllamaLimits::from_values(None, None).unwrap();
        assert_eq!(limits.num_ctx, 16384);
        assert_eq!(limits.num_predict, 4096);
        assert_eq!(limits.prompt_budget(), 12288);
    }
    #[test]
    fn limits_accept_valid_values() {
        let limits = OllamaLimits::from_values(Some("20480"), Some(" 2048 ")).unwrap();
        assert_eq!((limits.num_ctx, limits.num_predict), (20480, 2048));
    }
    #[test]
    fn invalid_limits_fail_loudly() {
        for (ctx, predict, needle) in [
            (
                Some("abc"),
                None,
                "OLLAMA_NUM_CTX must be a positive integer",
            ),
            (Some("0"), None, "OLLAMA_NUM_CTX must be a positive integer"),
            (
                Some("-5"),
                None,
                "OLLAMA_NUM_CTX must be a positive integer",
            ),
            (Some(""), None, "OLLAMA_NUM_CTX must be a positive integer"),
            (
                Some("1.5"),
                None,
                "OLLAMA_NUM_CTX must be a positive integer",
            ),
            (
                None,
                Some("lots"),
                "OLLAMA_NUM_PREDICT must be a positive integer",
            ),
            (
                None,
                Some("0"),
                "OLLAMA_NUM_PREDICT must be a positive integer",
            ),
            (
                Some("4096"),
                Some("4096"),
                "must be smaller than OLLAMA_NUM_CTX",
            ),
            (Some("2048"), None, "must be smaller than OLLAMA_NUM_CTX"),
            (
                Some("8192"),
                Some("9000"),
                "must be smaller than OLLAMA_NUM_CTX",
            ),
        ] {
            let error = OllamaLimits::from_values(ctx, predict).unwrap_err();
            assert!(error.contains(needle), "{ctx:?} {predict:?}: {error}");
        }
    }
    #[test]
    fn options_carry_context_and_output_cap() {
        let limits = OllamaLimits::new(16384, 4096).unwrap();
        assert_eq!(
            limits.options(Some(0)),
            json!({"num_ctx":16384,"num_predict":4096,"temperature":0})
        );
        assert_eq!(
            limits.options(None),
            json!({"num_ctx":16384,"num_predict":4096})
        );
    }
    #[test]
    fn token_estimate_rounds_up_and_counts_characters() {
        assert_eq!(estimate_tokens(""), 0);
        // 300 letters are 100 tokens, plus ten percent.
        assert_eq!(estimate_tokens(&"a".repeat(300)), 110);
        assert_eq!(estimate_tokens(&"é".repeat(300)), 110);
        assert_eq!(estimate_tokens(&"a".repeat(301)), 112);
        // Digits cost one token each.
        assert_eq!(estimate_tokens(&"7".repeat(100)), 110);
    }
    #[test]
    fn preflight_names_estimate_context_and_variable() {
        let limits = OllamaLimits::new(1000, 400).unwrap();
        assert!(limits.preflight(&"x".repeat(1635)).is_ok());
        let error = limits.preflight(&"x".repeat(1636)).unwrap_err().to_string();
        assert!(error.contains("601 tokens"), "{error}");
        assert!(error.contains("1000"), "{error}");
        assert!(error.contains("OLLAMA_NUM_CTX"), "{error}");
    }
    #[test]
    fn truncated_prompt_response_is_rejected() {
        let limits = OllamaLimits::new(16384, 4096).unwrap();
        for evaluated in [16384, 16383, 16368] {
            let error = limits
                .check_response(&json!({"prompt_eval_count":evaluated,"done_reason":"stop"}))
                .unwrap_err()
                .to_string();
            assert!(error.contains("prompt was truncated by Ollama"), "{error}");
        }
        assert!(
            limits
                .check_response(&json!({"prompt_eval_count":16367,"done_reason":"stop"}))
                .is_ok()
        );
        assert!(
            limits
                .check_response(&json!({"prompt_eval_count":9000,"done_reason":"stop"}))
                .is_ok()
        );
    }
    #[test]
    fn answer_cut_at_the_output_cap_is_rejected() {
        let limits = OllamaLimits::new(16384, 4096).unwrap();
        let error = limits
            .check_response(
                &json!({"prompt_eval_count":900,"eval_count":4096,"done_reason":"length"}),
            )
            .unwrap_err()
            .to_string();
        assert!(error.contains("OLLAMA_NUM_PREDICT"), "{error}");
    }
    #[test]
    fn response_without_counters_is_rejected() {
        let limits = OllamaLimits::new(16384, 4096).unwrap();
        let error = limits
            .check_response(&json!({"response":"{}","done":true}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("prompt_eval_count"), "{error}");
    }
    #[test]
    fn local_cache_identity_includes_context() {
        let local = OllamaJsonModel {
            base: "http://127.0.0.1:1".into(),
            model: "m".into(),
            limits: OllamaLimits::new(16384, 4096).unwrap(),
        };
        assert_eq!(local.identity(), "ollama/m");
        assert_eq!(
            local.cache_identity(),
            "ollama/m/ctx16384/nothink/structured-v3"
        );
    }
}
