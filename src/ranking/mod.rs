use crate::error::NexusError;
use crate::model::{RankingMode, ScoredPassage};
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
pub async fn rank_passages(
    query: &str,
    passages: &mut [ScoredPassage],
    mode: RankingMode,
    embedder: Option<&dyn TextEmbedder>,
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
            let metrics = hybrid::rank_hybrid(query, passages, emb).await?;
            Ok(RankingMetricsOutcome {
                sparse_ranking_ms: Some(metrics.sparse_ms),
                dense_ranking_ms: Some(metrics.dense_ms),
                rrf_fusion_ms: Some(metrics.rrf_ms),
                total_ranking_ms: start.elapsed().as_millis() as u64,
            })
        }
    }
}
