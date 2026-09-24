use async_trait::async_trait;

use crate::error::NexusError;

/// Asynchronous embedding interface allowing callers to provide dense vector scoring.
#[async_trait]
pub trait TextEmbedder: Send + Sync {
    /// Generates an embedding vector for a single string slice.
    async fn embed_text(&self, text: &str) -> Result<Vec<f32>, NexusError>;

    /// Generates embedding vectors for a batch of string slices.
    async fn embed_batch(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, NexusError>;
}
