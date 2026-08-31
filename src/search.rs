use serde::Serialize;
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct RankedCandidate {
    pub version_id: Uuid,
    pub statement: String,
    pub lexical_rank: Option<usize>,
    pub semantic_rank: Option<usize>,
    pub fused_score: f64,
}

pub fn reciprocal_rank_fusion(
    lexical: &[(Uuid, String)],
    semantic: &[(Uuid, String)],
    k: f64,
) -> Vec<RankedCandidate> {
    let mut map: HashMap<Uuid, RankedCandidate> = HashMap::new();
    for (i, (id, s)) in lexical.iter().enumerate() {
        let e = map.entry(*id).or_insert(RankedCandidate {
            version_id: *id,
            statement: s.clone(),
            lexical_rank: None,
            semantic_rank: None,
            fused_score: 0.0,
        });
        e.lexical_rank = Some(i + 1);
        e.fused_score += 1.0 / (k + (i + 1) as f64);
    }
    for (i, (id, s)) in semantic.iter().enumerate() {
        let e = map.entry(*id).or_insert(RankedCandidate {
            version_id: *id,
            statement: s.clone(),
            lexical_rank: None,
            semantic_rank: None,
            fused_score: 0.0,
        });
        e.semantic_rank = Some(i + 1);
        e.fused_score += 1.0 / (k + (i + 1) as f64);
    }
    let mut out: Vec<_> = map.into_values().collect();
    out.sort_by(|a, b| {
        b.fused_score
            .total_cmp(&a.fused_score)
            .then_with(|| a.version_id.cmp(&b.version_id))
    });
    out
}
pub fn estimate_tokens(text: &str) -> usize {
    text.chars().count().div_ceil(4)
}
pub fn pack(
    candidates: &[RankedCandidate],
    max_tokens: usize,
    top_k: usize,
) -> (Vec<RankedCandidate>, String, usize) {
    let mut chosen = Vec::new();
    let mut context = String::new();
    let mut used = 0;
    for item in candidates.iter().take(top_k) {
        let line = format!("- {}\n", item.statement);
        let cost = estimate_tokens(&line);
        if used + cost > max_tokens {
            continue;
        }
        used += cost;
        context.push_str(&line);
        chosen.push(item.clone());
    }
    (chosen, context, used)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_ceiling() {
        assert_eq!(estimate_tokens("12345"), 2)
    }
    #[test]
    fn rrf_prefers_overlap() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        let c = Uuid::new_v4();
        let r = reciprocal_rank_fusion(
            &[(a, "a".into()), (b, "b".into())],
            &[(c, "c".into()), (a, "a".into())],
            60.0,
        );
        assert_eq!(r[0].version_id, a)
    }
    #[test]
    fn packing_obeys_budget() {
        let c = RankedCandidate {
            version_id: Uuid::new_v4(),
            statement: "12345678".into(),
            lexical_rank: Some(1),
            semantic_rank: None,
            fused_score: 1.0,
        };
        let (_, _, n) = pack(&[c], 1, 5);
        assert_eq!(n, 0)
    }
}
