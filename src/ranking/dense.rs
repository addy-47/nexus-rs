use crate::error::NexusError;
use crate::model::ScoredPassage;
use crate::traits::TextEmbedder;

/// Computes dense cosine similarity scores for passages against the query in input order.
pub async fn score_dense(
    query: &str,
    passages: &[ScoredPassage],
    embedder: &dyn TextEmbedder,
) -> Result<Vec<f32>, NexusError> {
    if passages.is_empty() {
        return Ok(Vec::new());
    }

    let query_vector = embedder.embed_text(query).await?;
    if query_vector.is_empty() || query_vector.iter().any(|v| !v.is_finite()) {
        return Err(NexusError::Embedding(
            "Query embedding returned empty or non-finite vector".to_owned(),
        ));
    }

    let texts: Vec<&str> = passages.iter().map(|p| p.text.as_str()).collect();
    let doc_vectors = embedder.embed_batch(&texts).await?;

    if doc_vectors.len() != passages.len() {
        return Err(NexusError::Embedding(format!(
            "embed_batch returned {} vectors for {} passages",
            doc_vectors.len(),
            passages.len()
        )));
    }

    let mut scores = Vec::with_capacity(passages.len());
    for (idx, doc_vector) in doc_vectors.iter().enumerate() {
        if doc_vector.is_empty() || doc_vector.iter().any(|v| !v.is_finite()) {
            return Err(NexusError::Embedding(format!(
                "Passage {} embedding returned empty or non-finite vector",
                idx
            )));
        }
        let sim = cosine_similarity(&query_vector, doc_vector);
        scores.push(sim);
    }

    Ok(scores)
}

/// Scores and ranks passage chunks using dense vector cosine similarity.
pub async fn rank_dense(
    query: &str,
    passages: &mut [ScoredPassage],
    embedder: &dyn TextEmbedder,
) -> Result<(), NexusError> {
    if passages.is_empty() {
        return Ok(());
    }

    let scores = score_dense(query, passages, embedder).await?;
    for (idx, passage) in passages.iter_mut().enumerate() {
        let sim = scores[idx];
        passage.dense_score = Some(sim);
        passage.score = sim;
    }

    passages.sort_by(|a, b| b.score.total_cmp(&a.score));
    Ok(())
}

/// Computes cosine similarity between two floating point vectors.
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.is_empty() || a.len() != b.len() {
        return 0.0;
    }

    let mut dot_product = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for i in 0..a.len() {
        dot_product += a[i] * b[i];
        norm_a += a[i] * a[i];
        norm_b += b[i] * b[i];
    }

    let denominator = norm_a.sqrt() * norm_b.sqrt();
    if denominator <= 0.0 {
        return 0.0;
    }

    dot_product / denominator
}
