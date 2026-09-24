use crate::error::NexusError;
use crate::model::{RankingMode, ScoredPassage};
use crate::traits::TextEmbedder;

pub mod bm25;
pub mod dense;
pub mod hybrid;

/// Ranks candidate passages using the specified ranking mode.
pub async fn rank_passages(
    query: &str,
    passages: &mut [ScoredPassage],
    mode: RankingMode,
    embedder: Option<&dyn TextEmbedder>,
) -> Result<(), NexusError> {
    match mode {
        RankingMode::Sparse => {
            bm25::rank_bm25(query, passages);
            Ok(())
        }
        RankingMode::Dense => {
            let Some(emb) = embedder else {
                return Err(NexusError::InvalidConfiguration(
                    "Dense ranking mode requires a configured TextEmbedder".to_string(),
                ));
            };
            dense::rank_dense(query, passages, emb).await
        }
        RankingMode::Hybrid => {
            let Some(emb) = embedder else {
                return Err(NexusError::InvalidConfiguration(
                    "Hybrid ranking mode requires a configured TextEmbedder".to_string(),
                ));
            };
            hybrid::rank_hybrid(query, passages, emb).await
        }
    }
}
