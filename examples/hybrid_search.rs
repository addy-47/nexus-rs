use std::sync::Arc;

use async_trait::async_trait;
use nexus::{
    Engine, NexusError, NexusSearch, NexusSearchOptions, RankingMode, TextEmbedder, TimeFilter,
};

/// Demonstration embedder generating deterministic 384-dimensional unit vectors.
/// In production, this trait is implemented over your local ONNX runtime (ort/Candle) or API.
struct MockModelEmbedder {
    dimension: usize,
}

impl MockModelEmbedder {
    fn new(dimension: usize) -> Self {
        Self { dimension }
    }

    /// Computes a deterministic pseudo-embedding based on string character hashes.
    fn compute_vector(&self, text: &str) -> Vec<f32> {
        let mut vector = vec![0.0f32; self.dimension];
        for (i, byte) in text.as_bytes().iter().enumerate() {
            let slot = (i + *byte as usize) % self.dimension;
            vector[slot] += 1.0;
        }

        // Normalize to unit length for cosine similarity
        let norm: f32 = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
        if norm > 0.0 {
            for v in &mut vector {
                *v /= norm;
            }
        }
        vector
    }
}

#[async_trait]
impl TextEmbedder for MockModelEmbedder {
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, NexusError> {
        Ok(self.compute_vector(text))
    }

    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NexusError> {
        Ok(texts.iter().map(|t| self.compute_vector(t)).collect())
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let embedder = Arc::new(MockModelEmbedder::new(384));

    // 1. Configure the engine with our neural embedder handle
    let engine = NexusSearch::builder()
        .with_engines(vec![Engine::Duckduckgo, Engine::Bing])
        .with_embedder(embedder)
        .with_fetch_concurrency(3)
        .build()?;

    // 2. Select RankingMode::Hybrid to activate Reciprocal Rank Fusion (k=60)
    let options = NexusSearchOptions {
        time_filter: TimeFilter::Any,
        ranking_mode: RankingMode::Hybrid,
        max_candidates: 3,
        chunk_size_words: 150,
        chunk_overlap_words: 30,
        fetch_timeout_ms: 4000,
        max_response_bytes: 524_288,
    };

    println!("Executing hybrid RRF search for 'high performance rust async'...");
    let result = engine
        .search("high performance rust async", &options)
        .await?;

    println!("\nTop 5 Scored Passages (Hybrid RRF):");
    for (i, passage) in result.scored_passages.iter().take(5).enumerate() {
        println!(
            "\n[#{}] RRF Score: {:.5} (BM25: {:.2}, Dense: {:.3}) | {}",
            i + 1,
            passage.score,
            passage.sparse_score.unwrap_or(0.0),
            passage.dense_score.unwrap_or(0.0),
            passage.source_title
        );
        println!("URL: {}", passage.source_url);
        println!("Excerpt: {}", passage.text);
    }

    Ok(())
}
