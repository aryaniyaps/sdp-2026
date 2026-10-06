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
    for (field, value) in [
        ("subject.name", &c.subject.name),
        ("subject.entity_type", &c.subject.entity_type),
        ("predicate", &c.predicate),
        ("value", &c.value),
        ("statement", &c.statement),
    ] {
        if value.trim().is_empty() {
            return Err(Validation(format!(
                "claim fields must be nonempty: {field} is empty in claim \"{}\"",
                clip(&c.statement, 100)
            )));
        }
    }
    if !c.confidence.is_finite() || !(0.0..=1.0).contains(&c.confidence) {
        return Err(Validation(format!(
            "confidence must be finite and between zero and one, got {} in claim \"{}\"",
            c.confidence,
            clip(&c.statement, 100)
        )));
    }
    const ENTITY_TYPES: [&str; 6] = [
        "person",
        "organization",
        "project",
        "place",
        "technology",
        "other",
    ];
    for entity in std::iter::once(&c.subject).chain(&c.entities) {
        if entity.name.trim().is_empty() {
            return Err(Validation(format!(
                "an entity in claim \"{}\" has an empty name",
                clip(&c.statement, 100)
            )));
        }
        if !ENTITY_TYPES.contains(&entity.entity_type.as_str()) {
            return Err(Validation(format!(
                "entity \"{}\" has entity_type \"{}\", which is not allowed; use exactly one of {}",
                clip(&entity.name, 60),
                clip(&entity.entity_type, 40),
                ENTITY_TYPES.join(", ")
            )));
        }
    }
    for relation in &c.related {
        if !["extends", "contradicts", "causes"].contains(&relation.relation.as_str()) {
            return Err(Validation(format!(
                "related relation \"{}\" is not allowed; use exactly one of extends, contradicts, causes",
                clip(&relation.relation, 40)
            )));
        }
        if relation.explanation.trim().is_empty() {
            return Err(Validation(
                "every related edge needs a nonempty explanation".into(),
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
        return Err(Validation(format!(
            "claim kind \"{}\" is not allowed; use exactly one of fact, preference, profile, goal, episode, procedure, other",
            clip(&c.kind, 40)
        )));
    }
    if c.source_indices.is_empty() || c.source_indices.len() != c.quotes.len() {
        return Err(Validation(format!(
            "every claim needs paired source indices and exact quotes: claim \"{}\" has {} source_indices and {} quotes; they are parallel arrays of equal length, so repeat an event index once per quote when two quotes come from the same event",
            c.statement,
            c.source_indices.len(),
            c.quotes.len()
        )));
    }
    for (i, q) in c.source_indices.iter().zip(&c.quotes) {
        if q.trim().is_empty() {
            return Err(Validation(format!(
                "claim \"{}\" has an empty quote; every quote must be exact text copied from its event",
                clip(&c.statement, 100)
            )));
        }
        let Some(event) = events.get(*i) else {
            return Err(Validation(format!(
                "claim \"{}\" cites source index {i}, but there are {} events numbered from 0",
                clip(&c.statement, 100),
                events.len()
            )));
        };
        if !event.content.contains(q) {
            // Name the quote and the event: a model that is only told "not verbatim" tends to
            // repeat the same paraphrase, while a named quote and a hint let it copy the text.
            let hint = match events.iter().position(|e| e.content.contains(q)) {
                Some(j) => format!("that text does occur in event {j}, so use source index {j}"),
                None => "copy the exact characters from the event content, including punctuation, case and whitespace, or quote a shorter piece of it".to_string(),
            };
            return Err(Validation(format!(
                "claim \"{}\": quote \"{}\" does not occur verbatim in event {i}; {hint}",
                clip(&c.statement, 100),
                clip(q, 120)
            )));
        }
    }
    Ok(())
}
/// At most `max` characters of `text`, for error messages that go back to a model.
fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max).collect();
    out.push_str("...");
    out
}
#[cfg(test)]
mod claim_validation_tests {
    use super::*;
    fn event(content: &str) -> EvidenceEvent {
        EvidenceEvent {
            role: "user".into(),
            content: content.into(),
            occurred_at: Utc::now(),
            metadata: serde_json::json!({}),
        }
    }
    fn claim(indices: Vec<usize>, quotes: Vec<&str>) -> ClaimInput {
        serde_json::from_value(serde_json::json!({
            "subject":{"name":"Ada","entity_type":"person","aliases":[]},
            "predicate":"uses","value":"Rust","statement":"Ada uses Rust","cardinality":"single",
            "kind":"fact","confidence":0.9,"source_indices":indices,"quotes":quotes,
            "entities":[],"correction":false,"explanation":"x","related":[]
        }))
        .unwrap()
    }
    #[test]
    fn two_quotes_from_one_event_need_a_repeated_index() {
        let events = [event("Ada uses Rust and deploys on Fridays")];
        assert!(
            validate_claim(
                &claim(vec![0, 0], vec!["Ada uses Rust", "deploys on Fridays"]),
                &events
            )
            .is_ok()
        );
        let error = validate_claim(
            &claim(vec![0], vec!["Ada uses Rust", "deploys on Fridays"]),
            &events,
        )
        .unwrap_err()
        .to_string();
        assert!(error.contains("1 source_indices and 2 quotes"), "{error}");
        assert!(error.contains("repeat an event index"), "{error}");
    }
    #[test]
    fn a_quote_that_is_not_verbatim_is_named_with_its_event() {
        let events = [event("Ada uses Rust"), event("Grace prefers COBOL")];
        let error = validate_claim(&claim(vec![0], vec!["Ada likes Rust"]), &events)
            .unwrap_err()
            .to_string();
        assert!(error.contains("quote \"Ada likes Rust\""), "{error}");
        assert!(error.contains("event 0"), "{error}");
        assert!(error.contains("copy the exact characters"), "{error}");
    }
    #[test]
    fn a_quote_found_in_another_event_names_that_event() {
        let events = [event("Ada uses Rust"), event("Grace prefers COBOL")];
        let error = validate_claim(&claim(vec![0], vec!["Grace prefers COBOL"]), &events)
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("occur in event 1, so use source index 1"),
            "{error}"
        );
    }
    #[test]
    fn an_index_outside_the_events_and_an_empty_quote_are_reported() {
        let events = [event("Ada uses Rust")];
        let error = validate_claim(&claim(vec![3], vec!["Ada uses Rust"]), &events)
            .unwrap_err()
            .to_string();
        assert!(error.contains("source index 3"), "{error}");
        assert!(error.contains("1 events numbered from 0"), "{error}");
        let error = validate_claim(&claim(vec![0], vec!["  "]), &events)
            .unwrap_err()
            .to_string();
        assert!(error.contains("empty quote"), "{error}");
    }
    #[test]
    fn entity_kind_and_relation_errors_name_the_value_and_the_allowed_ones() {
        let events = [event("Ada uses Rust")];
        let mut bad_type = claim(vec![0], vec!["Ada uses Rust"]);
        bad_type.subject.entity_type = "language".into();
        let error = validate_claim(&bad_type, &events).unwrap_err().to_string();
        assert!(error.contains("entity_type \"language\""), "{error}");
        assert!(error.contains("person, organization, project"), "{error}");
        let mut bad_kind = claim(vec![0], vec!["Ada uses Rust"]);
        bad_kind.kind = "opinion".into();
        let error = validate_claim(&bad_kind, &events).unwrap_err().to_string();
        assert!(error.contains("kind \"opinion\""), "{error}");
        assert!(error.contains("fact, preference"), "{error}");
        let mut empty = claim(vec![0], vec!["Ada uses Rust"]);
        empty.value = " ".into();
        let error = validate_claim(&empty, &events).unwrap_err().to_string();
        assert!(error.contains("value is empty"), "{error}");
    }
    #[test]
    fn long_text_in_a_validation_message_is_clipped() {
        let long = "x".repeat(400);
        let events = [event("short event")];
        let error = validate_claim(&claim(vec![0], vec![long.as_str()]), &events)
            .unwrap_err()
            .to_string();
        assert!(error.len() < 600, "{}", error.len());
        assert!(error.contains("..."), "{error}");
        assert_eq!(clip("héllo", 2), "hé...");
    }
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
