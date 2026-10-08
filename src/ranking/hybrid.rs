use std::collections::HashSet;

use super::bm25::score_bm25;
use super::dense::score_dense;
use crate::engines::helpers::norm_url_key;
use crate::error::NexusError;
use crate::model::{RankingPolicy, ScoredPassage};
use crate::traits::TextEmbedder;

/// Telemetry metrics for hybrid ranking.
#[derive(Clone, Debug, Default)]
pub struct HybridRankingMetrics {
    /// Sparse BM25 scoring duration in milliseconds.
    pub sparse_ms: u64,
    /// Dense embedding and cosine similarity scoring duration in milliseconds.
    pub dense_ms: u64,
    /// Hybrid score combination duration in milliseconds.
    pub rrf_ms: u64,
}

/// Combines sparse and dense scoring using weighted combination of dense cosine similarity and normalized BM25 with 2-stage re-ranking.
pub async fn rank_hybrid(
    query: &str,
    passages: &mut [ScoredPassage],
    embedder: &dyn TextEmbedder,
    policy: &RankingPolicy,
    consensus_urls: Option<&HashSet<String>>,
) -> Result<HybridRankingMetrics, NexusError> {
    if passages.is_empty() {
        return Ok(HybridRankingMetrics::default());
    }

    let n = passages.len();

    // 1. Compute sparse BM25 scores in input order
    let sparse_start = std::time::Instant::now();
    let sparse_scores = score_bm25(query, passages);
    let sparse_ms = sparse_start.elapsed().as_millis() as u64;

    // 2. 2-Stage Re-ranking: Pre-filter candidate passages before dense neural embedding
    let dense_start = std::time::Instant::now();
    let dense_scores = if policy.two_stage_reranking && n > policy.max_candidates_to_rerank {
        let anchors = crate::extraction::quality::entity_anchors(query);
        let substantives = crate::extraction::quality::substantive_query_tokens(query);

        let mut sorted_indices: Vec<usize> = (0..n).collect();
        sorted_indices.sort_by(|&a, &b| {
            // Combine BM25 with anchor/title overlap to prevent dropping relevant single-sentence facts
            let calc_pre_score = |idx: usize| -> f32 {
                let mut s = sparse_scores[idx];
                let p = &passages[idx];
                let text_lower = p.text.to_ascii_lowercase();
                let title_lower = p.source_title.to_ascii_lowercase();
                for anchor in &anchors {
                    if text_lower.contains(anchor) {
                        s += 2.0;
                    }
                    if title_lower.contains(anchor) {
                        s += 1.0;
                    }
                }
                for sub in &substantives {
                    if text_lower.contains(sub) {
                        s += 0.5;
                    }
                }
                s
            };
            let score_a = calc_pre_score(a);
            let score_b = calc_pre_score(b);
            score_b.total_cmp(&score_a)
        });

        let top_k = policy.max_candidates_to_rerank.min(n);
        let top_indices = &sorted_indices[..top_k];

        let candidate_passages: Vec<ScoredPassage> = top_indices
            .iter()
            .map(|&idx| passages[idx].clone())
            .collect();
        let candidate_dense_scores = score_dense(query, &candidate_passages, embedder).await?;

        let mut all_dense_scores = vec![None; n];
        for (cand_i, &orig_idx) in top_indices.iter().enumerate() {
            all_dense_scores[orig_idx] = Some(candidate_dense_scores[cand_i]);
        }

        all_dense_scores
    } else {
        let full_dense = score_dense(query, passages, embedder).await?;
        full_dense.into_iter().map(Some).collect()
    };
    let dense_ms = dense_start.elapsed().as_millis() as u64;

    // 3. Compute direct hybrid scores combining dense cosine similarity and normalized BM25
    let rrf_start = std::time::Instant::now();
    let max_sparse = sparse_scores.iter().cloned().fold(0.0f32, f32::max);

    for idx in 0..n {
        let s_raw = sparse_scores[idx];
        let s_norm = if max_sparse > f32::EPSILON {
            (s_raw.max(0.0) / max_sparse).clamp(0.0, 1.0)
        } else {
            0.0
        };

        let d_opt = dense_scores[idx];
        let d_sim = d_opt.unwrap_or(0.0).clamp(0.0, 1.0);

        let mut hybrid_score = if d_opt.is_some() {
            // Weighted combination: 65% neural semantic similarity + 35% lexical BM25
            0.65 * d_sim + 0.35 * s_norm
        } else {
            s_norm
        };

        // Apply consensus corroboration boost if source URL was confirmed by >= 2 index families
        if let Some(urls) = consensus_urls {
            let key = norm_url_key(&passages[idx].source_url);
            if urls.contains(&key) {
                hybrid_score *= policy.consensus_multiplier;
            }
        }

        passages[idx].sparse_score = Some(s_raw);
        passages[idx].dense_score = d_opt;
        passages[idx].score = hybrid_score;
    }

    passages.sort_by(|a, b| b.score.total_cmp(&a.score));
    let rrf_ms = rrf_start.elapsed().as_millis() as u64;

    Ok(HybridRankingMetrics {
        sparse_ms,
        dense_ms,
        rrf_ms,
    })
}
