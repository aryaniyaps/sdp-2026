use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub struct ExtractedMemory {
    pub subject: String,
    pub predicate: String,
    pub value: String,
    pub statement: String,
    pub kind: MemoryKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKind {
    Preference,
    Fact,
    Profile,
    Goal,
    Other,
}

impl MemoryKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Fact => "fact",
            Self::Profile => "profile",
            Self::Goal => "goal",
            Self::Other => "other",
        }
    }
}

pub fn normalize_component(input: &str) -> String {
    input
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_lowercase()
}
pub fn canonical_key(subject: &str, predicate: &str) -> String {
    format!(
        "{}::{}",
        normalize_component(subject),
        normalize_component(predicate)
    )
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct VersionView {
    pub id: Uuid,
    pub memory_id: Uuid,
    pub canonical_key: String,
    pub subject: String,
    pub predicate: String,
    pub version: i32,
    pub value: String,
    pub statement: String,
    pub kind: String,
    pub status: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub extractor_version: String,
    pub sources: Vec<SourceView>,
    pub supersedes: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SourceView {
    pub chunk_id: Uuid,
    pub quote: String,
    pub role: String,
    pub session_id: Uuid,
    pub external_session_id: String,
    pub occurred_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResolveOutcome {
    pub memory_id: Uuid,
    pub version_id: Uuid,
    pub version: i32,
    pub action: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_normalization() {
        assert_eq!(
            canonical_key("  Aryan! ", "Prefers   Language."),
            "aryan::prefers language"
        );
    }
}

/// One page of canonical memories, each with its full version chain.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BrowsePage {
    pub total: i64,
    pub memories: Vec<Vec<VersionView>>,
}

/// Corpus-level counts for the console header.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct CorpusStats {
    pub memories: i64,
    pub versions: i64,
    pub superseded: i64,
    pub chains: i64,
    pub relations: i64,
    pub sessions: i64,
    pub events: i64,
    pub chunks: i64,
}
