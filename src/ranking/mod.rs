use std::collections::HashSet;

use crate::error::NexusError;
use crate::model::{RankingMode, RankingPolicy, ScoredPassage};
use crate::traits::TextEmbedder;

pub mod bm25;
pub mod dense;
pub mod hybrid;

/// Telemetry metrics produced during passage relevance ranking.
#[derive(Clone, Debug, Default)]
pub struct RankingMetricsOutcome {
    /// BM25 sparse scoring duration in milliseconds, if executed.
    pub sparse_ranking_ms: Option<u64>,
    /// Dense embedding and cosine similarity duration in milliseconds, if executed.
    pub dense_ranking_ms: Option<u64>,
    /// Reciprocal Rank Fusion calculation duration in milliseconds, if executed.
    pub rrf_fusion_ms: Option<u64>,
    /// Total passage ranking duration in milliseconds.
    pub total_ranking_ms: u64,
}

/// Ranks candidate passages using the specified ranking mode and returns timing metrics.
///
/// On return, `passages` are sorted best-first with min-max normalized scores in
/// [0, 1] (top passage = 1.0) and entries below `policy.min_score` removed.
/// P0-8: raw fused scores (e.g. RRF ≈ 0.033 by construction) carry no
/// discrimination signal and contradict the spec's `score="0.842"` example shape.
pub async fn rank_passages(
    query: &str,
    passages: &mut Vec<ScoredPassage>,
    mode: RankingMode,
    embedder: Option<&dyn TextEmbedder>,
    policy: &RankingPolicy,
    consensus_urls: Option<&HashSet<String>>,
) -> Result<RankingMetricsOutcome, NexusError> {
    let start = std::time::Instant::now();
    let outcome = rank_passages_inner(query, passages, mode, embedder, policy, consensus_urls).await?;
    normalize_and_floor(passages, policy.min_score);
    Ok(RankingMetricsOutcome {
        total_ranking_ms: start.elapsed().as_millis() as u64,
        ..outcome
    })
}

/// Min-max normalizes scores to [0, 1] (best = 1.0) and drops the tail below
/// `min_score`. No-op on empty input or uniform scores (all become 1.0).
fn normalize_and_floor(passages: &mut Vec<ScoredPassage>, min_score: f32) {
    if passages.is_empty() {
        return;
    }
    let (mut lo, mut hi) = (f32::INFINITY, f32::NEG_INFINITY);
    for p in passages.iter() {
        lo = lo.min(p.score);
        hi = hi.max(p.score);
    }
    let span = hi - lo;
    for p in passages.iter_mut() {
        p.score = if span > f32::EPSILON { (p.score - lo) / span } else { 1.0 };
    }
    if min_score > 0.0 {
        let before = passages.len();
        passages.retain(|p| p.score >= min_score);
        let dropped = before - passages.len();
        if dropped > 0 {
            log::info!(
                "[Nexus::Ranking] Relevance floor {min_score} rejected {dropped}/{before} passages"
            );
        }
    }
}

async fn rank_passages_inner(
    query: &str,
    passages: &mut [ScoredPassage],
    mode: RankingMode,
    embedder: Option<&dyn TextEmbedder>,
    policy: &RankingPolicy,
    consensus_urls: Option<&HashSet<String>>,
) -> Result<RankingMetricsOutcome, NexusError> {
    let start = std::time::Instant::now();
    match mode {
        RankingMode::Sparse => {
            let sparse_start = std::time::Instant::now();
            bm25::rank_bm25(query, passages);
            let sparse_ms = sparse_start.elapsed().as_millis() as u64;
            Ok(RankingMetricsOutcome {
                sparse_ranking_ms: Some(sparse_ms),
                dense_ranking_ms: None,
                rrf_fusion_ms: None,
                total_ranking_ms: start.elapsed().as_millis() as u64,
            })
        }
        RankingMode::Dense => {
            let Some(emb) = embedder else {
                return Err(NexusError::InvalidConfiguration(
                    "Dense ranking mode requires a configured TextEmbedder".to_string(),
                ));
            };
            let dense_start = std::time::Instant::now();
            dense::rank_dense(query, passages, emb).await?;
            let dense_ms = dense_start.elapsed().as_millis() as u64;
            Ok(RankingMetricsOutcome {
                sparse_ranking_ms: None,
                dense_ranking_ms: Some(dense_ms),
                rrf_fusion_ms: None,
                total_ranking_ms: start.elapsed().as_millis() as u64,
            })
        }
        RankingMode::Hybrid => {
            let Some(emb) = embedder else {
                return Err(NexusError::InvalidConfiguration(
                    "Hybrid ranking mode requires a configured TextEmbedder".to_string(),
                ));
            };
            let metrics = hybrid::rank_hybrid(query, passages, emb, policy, consensus_urls).await?;
            Ok(RankingMetricsOutcome {
                sparse_ranking_ms: Some(metrics.sparse_ms),
                dense_ranking_ms: Some(metrics.dense_ms),
                rrf_fusion_ms: Some(metrics.rrf_ms),
                total_ranking_ms: start.elapsed().as_millis() as u64,
            })
        }
    }
}
