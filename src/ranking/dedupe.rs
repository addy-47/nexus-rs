use std::collections::HashSet;

use crate::model::ScoredPassage;

pub const DEFAULT_DEDUPE_JACCARD_THRESHOLD: f32 = 0.80;

/// Filters out near-duplicate passages using pairwise Jaccard word-set similarity.
///
/// Passages should be provided in descending score order (best-first), ensuring
/// the highest-scoring passage in any cluster of near-duplicates is retained.
pub fn deduplicate_passages(passages: Vec<ScoredPassage>, threshold: f32) -> Vec<ScoredPassage> {
    if passages.len() <= 1 {
        return passages;
    }

    let mut retained: Vec<ScoredPassage> = Vec::with_capacity(passages.len());
    let mut token_sets: Vec<HashSet<String>> = Vec::with_capacity(passages.len());

    for candidate in passages {
        let cand_tokens: HashSet<String> = candidate
            .text
            .split_whitespace()
            .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase())
            .filter(|w| w.len() >= 2)
            .collect();

        if cand_tokens.is_empty() {
            retained.push(candidate);
            token_sets.push(cand_tokens);
            continue;
        }

        let is_duplicate = token_sets.iter().any(|existing| {
            let intersection = cand_tokens.intersection(existing).count();
            let union = cand_tokens.union(existing).count();
            if union == 0 {
                false
            } else {
                (intersection as f32 / union as f32) >= threshold
            }
        });

        if !is_duplicate {
            token_sets.push(cand_tokens);
            retained.push(candidate);
        }
    }

    retained
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_passage(text: &str, score: f32) -> ScoredPassage {
        ScoredPassage {
            text: text.to_string(),
            source_url: "https://example.com".to_string(),
            source_title: "Test".to_string(),
            passage_index: 0,
            score,
            sparse_score: None,
            dense_score: None,
        }
    }

    #[test]
    fn test_deduplicate_identical_and_near_identical() {
        let p1 = dummy_passage("Rust is a systems programming language focused on safety and speed.", 0.95);
        let p2 = dummy_passage("Rust is a systems programming language focused on safety and speed!", 0.90);
        let p3 = dummy_passage("Python is a dynamic programming language focused on readability.", 0.80);

        let input = vec![p1, p2, p3];
        let deduped = deduplicate_passages(input, DEFAULT_DEDUPE_JACCARD_THRESHOLD);

        assert_eq!(deduped.len(), 2);
        assert_eq!(deduped[0].score, 0.95);
        assert_eq!(deduped[1].score, 0.80);
    }
}
