use super::bm25::rank_bm25;
use super::dense::rank_dense;
use crate::error::NexusError;
use crate::model::ScoredPassage;
use crate::traits::TextEmbedder;

const RRF_K: f32 = 60.0;

/// Combines sparse and dense scoring using Reciprocal Rank Fusion (RRF with k=60).
pub async fn rank_hybrid(
    query: &str,
    passages: &mut [ScoredPassage],
    embedder: &dyn TextEmbedder,
) -> Result<(), NexusError> {
    if passages.is_empty() {
        return Ok(());
    }

    let n = passages.len();

    // 1. Sparse ranking pass
    let sparse_passages: Vec<(usize, ScoredPassage)> =
        passages.iter().cloned().enumerate().collect();
    let mut sparse_inner: Vec<ScoredPassage> =
        sparse_passages.iter().map(|(_, p)| p.clone()).collect();
    rank_bm25(query, &mut sparse_inner);

    let mut sparse_ranks = vec![0usize; n];
    let mut sparse_scores = vec![0.0f32; n];
    for (rank, p) in sparse_inner.into_iter().enumerate() {
        if let Some((orig_idx, _)) = sparse_passages
            .iter()
            .find(|(_, orig)| orig.passage_index == p.passage_index && orig.source_url == p.source_url)
        {
            sparse_ranks[*orig_idx] = rank + 1;
            sparse_scores[*orig_idx] = p.sparse_score.unwrap_or(0.0);
        }
    }

    // 2. Dense ranking pass
    let dense_passages: Vec<(usize, ScoredPassage)> =
        passages.iter().cloned().enumerate().collect();
    let mut dense_inner: Vec<ScoredPassage> =
        dense_passages.iter().map(|(_, p)| p.clone()).collect();
    rank_dense(query, &mut dense_inner, embedder).await?;

    let mut dense_ranks = vec![0usize; n];
    let mut dense_scores = vec![0.0f32; n];
    for (rank, p) in dense_inner.into_iter().enumerate() {
        if let Some((orig_idx, _)) = dense_passages
            .iter()
            .find(|(_, orig)| orig.passage_index == p.passage_index && orig.source_url == p.source_url)
        {
            dense_ranks[*orig_idx] = rank + 1;
            dense_scores[*orig_idx] = p.dense_score.unwrap_or(0.0);
        }
    }

    // 3. Compute RRF scores
    for (idx, passage) in passages.iter_mut().enumerate() {
        let s_rank = sparse_ranks[idx] as f32;
        let d_rank = dense_ranks[idx] as f32;

        let rrf = (1.0 / (RRF_K + s_rank)) + (1.0 / (RRF_K + d_rank));
        passage.sparse_score = Some(sparse_scores[idx]);
        passage.dense_score = Some(dense_scores[idx]);
        passage.score = rrf;
    }

    passages.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(())
}
