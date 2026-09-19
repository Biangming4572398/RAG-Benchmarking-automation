use serde::{Deserialize, Serialize};

/// The paired context is the single positive document. Repeated chunks keep
/// their original ranks and earn credit only at the first matching position.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
pub struct Scores {
    pub context_hit_at_k: f64,
    pub reciprocal_rank_at_k: f64,
    pub ndcg_at_k: f64,
}

pub fn score_context(expected_source: &str, ranked_sources: &[String], top_k: usize) -> Scores {
    let rank = ranked_sources
        .iter()
        .take(top_k)
        .position(|source| source == expected_source);
    match rank {
        Some(index) => Scores {
            context_hit_at_k: 1.0,
            reciprocal_rank_at_k: 1.0 / (index + 1) as f64,
            ndcg_at_k: 1.0 / ((index + 2) as f64).log2(),
        },
        None => Scores::default(),
    }
}

impl Scores {
    pub fn add(&mut self, other: Self) {
        self.context_hit_at_k += other.context_hit_at_k;
        self.reciprocal_rank_at_k += other.reciprocal_rank_at_k;
        self.ndcg_at_k += other.ndcg_at_k;
    }

    pub fn mean(self, count: usize) -> Option<Self> {
        (count > 0).then(|| Self {
            context_hit_at_k: self.context_hit_at_k / count as f64,
            reciprocal_rank_at_k: self.reciprocal_rank_at_k / count as f64,
            ndcg_at_k: self.ndcg_at_k / count as f64,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_are_not_promoted_when_a_document_has_multiple_chunks() {
        let sources = ["b", "b", "a", "a"].map(String::from);
        let score = score_context("a", &sources, 4);
        assert_eq!(score.context_hit_at_k, 1.0);
        assert!((score.reciprocal_rank_at_k - 0.3333333333333333).abs() < 1e-12);
        assert_eq!(score.ndcg_at_k, 0.5);
        assert_eq!(score_context("a", &sources, 2).context_hit_at_k, 0.0);
        assert_eq!(score_context("a", &[], 4).context_hit_at_k, 0.0);
    }
}
