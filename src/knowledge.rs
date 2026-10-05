//! Evidence-backed graph domain. Assertions are not arbitrary mutable text blobs.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RetainRequest {
    pub namespace: String,
    pub session_id: String,
    pub external_id: String,
    #[serde(default)]
    pub metadata: Value,
    pub events: Vec<EvidenceEvent>,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EvidenceEvent {
    pub role: String,
    pub content: String,
    pub occurred_at: DateTime<Utc>,
    #[serde(default)]
    pub metadata: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EntityInput {
    pub name: String,
    pub entity_type: String,
    #[serde(default)]
    pub aliases: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Cardinality {
    Single,
    Multiple,
    Event,
}
impl Cardinality {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Single => "single",
            Self::Multiple => "multiple",
            Self::Event => "event",
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ClaimInput {
    pub subject: EntityInput,
    pub predicate: String,
    pub value: String,
    pub statement: String,
    pub cardinality: Cardinality,
    pub kind: String,
    pub confidence: f32,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub valid_from: Option<DateTime<Utc>>,
    #[serde(default, deserialize_with = "optional_timestamp")]
    pub event_at: Option<DateTime<Utc>>,
    pub source_indices: Vec<usize>,
    pub quotes: Vec<String>,
    #[serde(default)]
    pub entities: Vec<EntityInput>,
    #[serde(default)]
    pub correction: bool,
    #[serde(default)]
    pub explanation: String,
    #[serde(default)]
    pub related: Vec<RelatedInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct RelatedInput {
    pub assertion_id: Uuid,
    pub relation: String,
    pub explanation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Extraction {
    pub claims: Vec<ClaimInput>,
}
fn optional_timestamp<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<DateTime<Utc>>, D::Error> {
    let value = Option::<String>::deserialize(deserializer)?;
    value
        .map(|value| {
            DateTime::parse_from_rfc3339(&value)
                .map(|date| date.with_timezone(&Utc))
                .or_else(|_| {
                    chrono::NaiveDate::parse_from_str(&value, "%Y-%m-%d")
                        .map(|date| date.and_hms_opt(0, 0, 0).unwrap().and_utc())
                })
                .map_err(serde::de::Error::custom)
        })
        .transpose()
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct ObservationInput {
    pub subject_id: Uuid,
    pub statement: String,
    pub predicate: String,
    pub value: String,
    pub confidence: f32,
    pub supports: Vec<Uuid>,
    pub explanation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Consolidation {
    pub observations: Vec<ObservationInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct AssertionView {
    pub id: Uuid,
    pub namespace: String,
    pub subject_id: Uuid,
    pub subject: String,
    pub predicate: String,
    pub value: String,
    pub statement: String,
    pub kind: String,
    pub status: String,
    pub cardinality: String,
    pub valid_from: DateTime<Utc>,
    pub valid_to: Option<DateTime<Utc>>,
    pub event_at: Option<DateTime<Utc>>,
    pub recorded_at: DateTime<Utc>,
    pub confidence: f32,
    pub sources: Vec<EvidenceSource>,
    pub relations: Vec<RelatedInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct EvidenceSource {
    pub chunk_id: Uuid,
    pub quote: String,
    pub role: String,
    pub session_id: String,
    pub occurred_at: DateTime<Utc>,
    pub metadata: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Job {
    pub id: Uuid,
    pub namespace: String,
    pub kind: String,
    pub payload: Value,
    pub status: String,
    pub attempts: i32,
    pub lease_token: Option<Uuid>,
    pub result: Option<Value>,
    pub error: Option<String>,
}

pub fn validate_claim(c: &ClaimInput, events: &[EvidenceEvent]) -> Result<(), crate::AppError> {
    use crate::AppError::Validation;
    if [
        &c.subject.name,
        &c.subject.entity_type,
        &c.predicate,
        &c.value,
        &c.statement,
    ]
    .iter()
    .any(|s| s.trim().is_empty())
    {
        return Err(Validation("claim fields must be nonempty".into()));
    }
    if !c.confidence.is_finite() || !(0.0..=1.0).contains(&c.confidence) {
        return Err(Validation(
            "confidence must be finite and between zero and one".into(),
        ));
    }
    for entity in std::iter::once(&c.subject).chain(&c.entities) {
        if entity.name.trim().is_empty()
            || ![
                "person",
                "organization",
                "project",
                "place",
                "technology",
                "other",
            ]
            .contains(&entity.entity_type.as_str())
        {
            return Err(Validation("invalid entity name or type".into()));
        }
    }
    for relation in &c.related {
        if !["extends", "contradicts", "causes"].contains(&relation.relation.as_str())
            || relation.explanation.trim().is_empty()
        {
            return Err(Validation(
                "related edges require an allowed relation and explanation".into(),
            ));
        }
    }
    if ![
        "fact",
        "preference",
        "profile",
        "goal",
        "episode",
        "procedure",
        "other",
    ]
    .contains(&c.kind.as_str())
    {
        return Err(Validation("invalid directly extracted memory kind".into()));
    }
    if c.source_indices.is_empty() || c.source_indices.len() != c.quotes.len() {
        return Err(Validation(
            "every claim needs paired source indices and exact quotes".into(),
        ));
    }
    for (i, q) in c.source_indices.iter().zip(&c.quotes) {
        if q.trim().is_empty() || events.get(*i).is_none_or(|e| !e.content.contains(q)) {
            return Err(Validation(
                "quote must occur verbatim in its attributed source".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;
    #[derive(Deserialize)]
    struct DateFixture {
        #[serde(deserialize_with = "optional_timestamp")]
        date: Option<DateTime<Utc>>,
    }
    #[test]
    fn date_only_and_offset_sources_preserve_a_defined_utc_time() {
        let day: DateFixture = serde_json::from_str(r#"{"date":"2026-03-10"}"#).unwrap();
        assert_eq!(day.date.unwrap().to_rfc3339(), "2026-03-10T00:00:00+00:00");
        let offset: DateFixture =
            serde_json::from_str(r#"{"date":"2026-03-10T05:30:00+05:30"}"#).unwrap();
        assert_eq!(day.date, offset.date);
        assert!(serde_json::from_str::<DateFixture>(r#"{"date":"yesterday"}"#).is_err());
        assert!(serde_json::from_str::<DateFixture>(r#"{"date":"2026-02-30"}"#).is_err());
    }
}
