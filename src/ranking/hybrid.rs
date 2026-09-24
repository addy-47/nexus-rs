use std::collections::HashSet;

use super::bm25::score_bm25;
use super::dense::score_dense;
use crate::engines::helpers::norm_url_key;
use crate::error::NexusError;
use crate::model::{RankingPolicy, ScoredPassage};
use crate::traits::TextEmbedder;

const RRF_K: f32 = 60.0;

/// Telemetry metrics for Reciprocal Rank Fusion hybrid ranking.
#[derive(Clone, Debug, Default)]
pub struct HybridRankingMetrics {
    /// Sparse BM25 scoring duration in milliseconds.
    pub sparse_ms: u64,
    /// Dense embedding and cosine similarity scoring duration in milliseconds.
    pub dense_ms: u64,
    /// Reciprocal Rank Fusion calculation duration in milliseconds.
    pub rrf_ms: u64,
}

/// Combines sparse and dense scoring using Reciprocal Rank Fusion (RRF with k=60) with 2-stage re-ranking.
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

    let sparse_ranks = compute_ranks(&sparse_scores);

    // 2. 2-Stage Re-ranking: Pre-filter candidate passages before dense neural embedding
    let dense_start = std::time::Instant::now();
    let (dense_scores, dense_ranks) = if policy.two_stage_reranking && n > policy.max_candidates_to_rerank {
        let mut sorted_indices: Vec<usize> = (0..n).collect();
        sorted_indices.sort_by(|&a, &b| sparse_scores[b].total_cmp(&sparse_scores[a]));

        let top_k = policy.max_candidates_to_rerank.min(n);
        let top_indices = &sorted_indices[..top_k];

        let candidate_passages: Vec<ScoredPassage> =
            top_indices.iter().map(|&idx| passages[idx].clone()).collect();
        let candidate_dense_scores = score_dense(query, &candidate_passages, embedder).await?;
        let candidate_ranks = compute_ranks(&candidate_dense_scores);

        let mut all_dense_scores = vec![None; n];
        let mut all_dense_ranks = vec![top_k + 1; n];

        for (cand_i, &orig_idx) in top_indices.iter().enumerate() {
            all_dense_scores[orig_idx] = Some(candidate_dense_scores[cand_i]);
            all_dense_ranks[orig_idx] = candidate_ranks[cand_i];
        }

        (all_dense_scores, all_dense_ranks)
    } else {
        let full_dense = score_dense(query, passages, embedder).await?;
        let full_ranks = compute_ranks(&full_dense);
        (full_dense.into_iter().map(Some).collect(), full_ranks)
    };
    let dense_ms = dense_start.elapsed().as_millis() as u64;

    // 3. Compute RRF scores directly into passages with consensus multiplier
    let rrf_start = std::time::Instant::now();
    for idx in 0..n {
        let s_rank = sparse_ranks[idx] as f32;
        let d_rank = dense_ranks[idx] as f32;

        let mut rrf = (1.0 / (RRF_K + s_rank)) + (1.0 / (RRF_K + d_rank));

        // Apply consensus corroboration boost if source URL was confirmed by >= 2 index families
        if let Some(urls) = consensus_urls {
            let key = norm_url_key(&passages[idx].source_url);
            if urls.contains(&key) {
                rrf *= policy.consensus_multiplier;
            }
        }

        passages[idx].sparse_score = Some(sparse_scores[idx]);
        passages[idx].dense_score = dense_scores[idx];
        passages[idx].score = rrf;
    }

    passages.sort_by(|a, b| b.score.total_cmp(&a.score));
    let rrf_ms = rrf_start.elapsed().as_millis() as u64;

    Ok(HybridRankingMetrics {
        sparse_ms,
        dense_ms,
        rrf_ms,
    })
}

/// Converts a slice of scores into 1-based ranks (1 = highest score).
fn compute_ranks(scores: &[f32]) -> Vec<usize> {
    let mut indices: Vec<usize> = (0..scores.len()).collect();
    indices.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));

    let mut ranks = vec![0usize; scores.len()];
    for (rank_0, &orig_idx) in indices.iter().enumerate() {
        ranks[orig_idx] = rank_0 + 1;
    }
    ranks
}
