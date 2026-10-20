//! Check update intent separately from extraction, and ground missing entity mentions.
//! The local extractor can mistake any differing value for an explicit correction. A narrow
//! second question must confirm replacement language before a correction may retire a fact.

use super::*;
use serde::Deserialize;

const REVIEW_VERSION: &str = "claim-review-v2";
const TEMPLATE: &str = include_str!("review_prompt.txt");

#[derive(Deserialize)]
struct Reviews {
    reviews: Vec<Review>,
}
#[derive(Deserialize)]
struct Review {
    claim_index: usize,
    explicit_change: bool,
    change_quote: String,
    entities: Vec<EntityInput>,
}

pub(super) async fn review(
    state: &AppState,
    extraction: Extraction,
    events: &[EvidenceEvent],
) -> Result<Extraction, AppError> {
    if extraction.claims.is_empty() {
        return Ok(extraction);
    }
    let claims: Vec<_> = extraction
        .claims
        .iter()
        .map(|claim| {
            json!({
                "subject": claim.subject.name,
                "predicate": claim.predicate,
                "value": claim.value,
                "statement": claim.statement,
                "correction": claim.correction,
                "quotes": claim.quotes,
            })
        })
        .collect();
    let prompt = fill_template(
        TEMPLATE.trim_end(),
        &[("CLAIMS", &serde_json::to_string(&claims).unwrap())],
    );
    let mut input = prompt.clone();
    for attempt in 0..2 {
        let output =
            cached_generate(&state.store, state.model.as_ref(), REVIEW_VERSION, &input).await?;
        match apply_review(&extraction, output, events) {
            Ok(reviewed) => return Ok(reviewed),
            Err(error) => {
                forget_generated(&state.store, state.model.as_ref(), REVIEW_VERSION, &input)
                    .await?;
                if attempt == 1 {
                    return Err(error);
                }
                input = format!(
                    "{prompt}\nValidation error: {error}. Return all reviews with exact source text and indices in order."
                );
            }
        }
    }
    unreachable!("review loop always returns")
}

fn apply_review(
    extraction: &Extraction,
    output: Value,
    events: &[EvidenceEvent],
) -> Result<Extraction, AppError> {
    let invalid = |message: &str| AppError::Provider(format!("claim review: {message}"));
    let reviews: Reviews =
        serde_json::from_value(output).map_err(|_| invalid("invalid review schema"))?;
    if reviews.reviews.len() != extraction.claims.len() {
        return Err(invalid("one review is required for every claim"));
    }
    let mut result = extraction.clone();
    for (index, (claim, review)) in result.claims.iter_mut().zip(reviews.reviews).enumerate() {
        if review.claim_index != index {
            return Err(invalid("claim indices must be unique and in order"));
        }
        if review.explicit_change {
            if review.change_quote.trim().is_empty()
                || !claim
                    .quotes
                    .iter()
                    .any(|q| q.contains(&review.change_quote))
            {
                return Err(invalid(
                    "change evidence must be an exact nonempty source quote",
                ));
            }
        } else if !review.change_quote.is_empty() {
            return Err(invalid("a non-correction must have empty change evidence"));
        }
        // Review may veto a proposed correction, never promote an ordinary claim to one.
        if claim.correction && !review.explicit_change {
            claim.correction = false;
            claim.explanation = "Conflicting report; source does not confirm replacement.".into();
            for relation in &mut claim.related {
                if relation.relation == "contradicts" {
                    relation.explanation = claim.explanation.clone();
                }
            }
        }
        for entity in review.entities {
            if entity.name.trim().is_empty()
                || !claim.quotes.iter().any(|q| q.contains(&entity.name))
                || !entity.aliases.is_empty()
            {
                // An optional enrichment must not block the namespace's otherwise valid
                // evidence. Keep the original claim and omit this unsupported suggestion.
                tracing::warn!("ignored an ungrounded entity suggestion from claim review");
                continue;
            }
            // Keep the extractor's subject identity and existing mentions; a review only
            // adds grounded names, never splits an existing entity on a proposed type change.
            if !std::iter::once(&claim.subject)
                .chain(&claim.entities)
                .any(|e| e.name.eq_ignore_ascii_case(&entity.name))
            {
                claim.entities.push(entity);
            }
        }
        validate_claim(claim, events)?;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(text: &str, correction: bool) -> (Extraction, Vec<EvidenceEvent>) {
        let events = vec![EvidenceEvent {
            role: "user".into(),
            content: text.into(),
            occurred_at: chrono::Utc::now(),
            metadata: Value::Null,
        }];
        let extraction: Extraction = serde_json::from_value(json!({"claims":[{
            "subject":{"name":"Orion","entity_type":"project","aliases":[]},
            "predicate":"database","value":"MySQL","statement":"Orion uses MySQL.",
            "cardinality":"single","kind":"fact","confidence":0.98,
            "source_indices":[0],"quotes":[text],"entities":[],"correction":correction,
            "explanation":"source","related":[]
        }]}))
        .unwrap();
        (extraction, events)
    }
    fn output(explicit: bool, quote: &str, name: &str) -> Value {
        json!({"reviews":[{"claim_index":0,"explicit_change":explicit,"change_quote":quote,
            "entities":[{"name":name,"entity_type":"technology"}]}]})
    }

    #[test]
    fn conflicting_report_loses_correction_and_gains_grounded_entity() {
        let (claims, events) = fixture("Orion uses MySQL.", true);
        let reviewed = apply_review(&claims, output(false, "", "MySQL"), &events).unwrap();
        assert!(!reviewed.claims[0].correction);
        assert_eq!(reviewed.claims[0].entities[0].name, "MySQL");
        assert!(claims.claims[0].correction, "input must remain untouched");
    }

    #[test]
    fn explicit_correction_requires_quoted_change_and_cannot_be_invented() {
        let text = "Correction: Orion uses MySQL, not PostgreSQL.";
        let (claims, events) = fixture(text, true);
        assert!(
            apply_review(&claims, output(true, text, "MySQL"), &events)
                .unwrap()
                .claims[0]
                .correction
        );
        assert!(apply_review(&claims, output(true, "", "MySQL"), &events).is_err());
        assert!(
            apply_review(
                &claims,
                output(true, "We migrated to MySQL.", "MySQL"),
                &events
            )
            .is_err()
        );
        let (ordinary, events) = fixture(text, false);
        assert!(
            !apply_review(&ordinary, output(true, text, "MySQL"), &events)
                .unwrap()
                .claims[0]
                .correction
        );
    }

    #[test]
    fn missing_reviews_and_wrong_indices_fail_but_invented_mentions_are_discarded() {
        let (claims, events) = fixture("Orion uses MySQL.", false);
        assert!(apply_review(&claims, json!({"reviews":[]}), &events).is_err());
        let mut wrong = output(false, "", "MySQL");
        wrong["reviews"][0]["claim_index"] = json!(1);
        assert!(apply_review(&claims, wrong, &events).is_err());
        let reviewed = apply_review(&claims, output(false, "", "SQLite"), &events).unwrap();
        assert!(reviewed.claims[0].entities.is_empty());
        assert_eq!(reviewed.claims[0].statement, claims.claims[0].statement);
    }
}
