use std::collections::HashSet;

use crate::error::NexusError;
use crate::model::{RankingMode, RankingPolicy, ScoredPassage};
use crate::traits::TextEmbedder;

pub mod bm25;
pub mod dedupe;
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
/// On return, `passages` are sorted best-first with calibrated scores in [0, 1],
/// entries below `policy.min_score` removed, and near-duplicate passages deduplicated.
pub async fn rank_passages(
    query: &str,
    passages: &mut Vec<ScoredPassage>,
    mode: RankingMode,
    embedder: Option<&dyn TextEmbedder>,
    policy: &RankingPolicy,
    consensus_urls: Option<&HashSet<String>>,
) -> Result<RankingMetricsOutcome, NexusError> {
    let start = std::time::Instant::now();
    let outcome =
        rank_passages_inner(query, passages, mode, embedder, policy, consensus_urls).await?;
    apply_entity_coverage_penalty(query, passages);
    passages.sort_by(|a, b| b.score.total_cmp(&a.score));
    calibrate_and_filter(passages, mode, policy);
    Ok(RankingMetricsOutcome {
        total_ranking_ms: start.elapsed().as_millis() as u64,
        ..outcome
    })
}

/// DonSeTch entity-coverage penalty:
/// When a query contains concrete anchor entities (e.g. versions "3.13", years "2024",
/// model identifiers "llama-3", or quoted spans), passages whose title and text fail
/// to cover any query anchor receive a 0.3x score penalty.
/// Abstract queries with zero anchors incur zero penalty.
pub fn apply_entity_coverage_penalty(query: &str, passages: &mut [ScoredPassage]) {
    let anchors = crate::extraction::quality::entity_anchors(query);
    if anchors.is_empty() {
        return;
    }

    for p in passages.iter_mut() {
        let haystack = format!("{} {}", p.source_title, p.text).to_ascii_lowercase();
        let matches_any = anchors.iter().any(|a| haystack.contains(a));
        if !matches_any {
            p.score *= 0.3;
        }
    }
}

/// Calibrates relevance scores to [0, 1] without min-max distortion, enforces the min_score
/// relevance floor, and deduplicates near-duplicate passages.
fn calibrate_and_filter(passages: &mut Vec<ScoredPassage>, mode: RankingMode, policy: &RankingPolicy) {
    if passages.is_empty() {
        return;
    }

    match mode {
        RankingMode::Sparse => {
            // Zero-anchored scaling for BM25 scores: preserves zero baseline so non-matching
            // passages stay at 0.0 while top passage scales to 1.0.
            let hi = passages
                .iter()
                .map(|p| p.score)
                .fold(f32::NEG_INFINITY, f32::max);
            if hi > f32::EPSILON {
                for p in passages.iter_mut() {
                    p.score = (p.score.max(0.0) / hi).clamp(0.0, 1.0);
                }
            } else {
                for p in passages.iter_mut() {
                    p.score = 1.0;
                }
            }
        }
        RankingMode::Dense => {
            // Cosine similarity is an absolute semantic score in [-1.0, 1.0].
            // Clamp negative similarity to 0.0.
            for p in passages.iter_mut() {
                p.score = p.score.clamp(0.0, 1.0);
            }
        }
        RankingMode::Hybrid => {
            // Calibrate RRF score against theoretical maximum (rank 1 in both sparse and dense,
            // with consensus multiplier).
            let max_rrf = (1.0 / 61.0 + 1.0 / 61.0) * policy.consensus_multiplier;
            if max_rrf > f32::EPSILON {
                for p in passages.iter_mut() {
                    p.score = (p.score / max_rrf).clamp(0.0, 1.0);
                }
            }
        }
    }

    // Enforce absolute min_score floor
    if policy.min_score <= 0.0 {
        log::warn!(
            "[Nexus::Ranking] min_score is {} — relevance floor DISABLED. Normalized noise will be delivered as evidence.",
            policy.min_score
        );
    } else {
        let before = passages.len();
        passages.retain(|p| p.score >= policy.min_score);
        let dropped = before - passages.len();
        if dropped > 0 {
            log::info!(
                "[Nexus::Ranking] Relevance floor {} rejected {}/{} passages",
                policy.min_score,
                dropped,
                before
            );
        }
    }

    // Deduplicate near-duplicate passages
    let deduped = dedupe::deduplicate_passages(
        std::mem::take(passages),
        dedupe::DEFAULT_DEDUPE_JACCARD_THRESHOLD,
    );
    *passages = deduped;
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
