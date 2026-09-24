use super::bm25::score_bm25;
use super::dense::score_dense;
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

    // 1. Compute sparse BM25 scores in input order
    let sparse_scores = score_bm25(query, passages);

    // 2. Compute dense cosine similarity scores in input order
    let dense_scores = score_dense(query, passages, embedder).await?;

    // 3. Compute 1-based ranks from scores in O(n log n)
    let sparse_ranks = compute_ranks(&sparse_scores);
    let dense_ranks = compute_ranks(&dense_scores);

    // 4. Compute RRF scores directly into passages without cloning or metadata searching
    for idx in 0..n {
        let s_rank = sparse_ranks[idx] as f32;
        let d_rank = dense_ranks[idx] as f32;

        let rrf = (1.0 / (RRF_K + s_rank)) + (1.0 / (RRF_K + d_rank));
        passages[idx].sparse_score = Some(sparse_scores[idx]);
        passages[idx].dense_score = Some(dense_scores[idx]);
        passages[idx].score = rrf;
    }

    passages.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(())
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
