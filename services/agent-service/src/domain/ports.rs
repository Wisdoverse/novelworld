use async_trait::async_trait;
use anyhow::Result;
use uuid::Uuid;

use crate::domain::entities::memory::ChatMessage;

/// Port for short-term message caching (Redis or similar).
/// Domain services depend on this trait, not on concrete cache implementations.
#[async_trait]
pub trait MessageCache: Send + Sync {
    async fn get_recent_messages(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ChatMessage>>;

    async fn push_message(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        msg: &ChatMessage,
    ) -> Result<()>;

    async fn clear(
        &self,
        character_id: Uuid,
        user_id: Uuid,
    ) -> Result<()>;
}

/// Port for LLM text summarization.
/// Domain services depend on this trait, not on concrete LLM clients.
#[async_trait]
pub trait TextSummarizer: Send + Sync {
    async fn summarize(&self, system: &str, text: &str) -> Result<String>;
}

/// Port for generating vector embeddings from text.
/// Used by the memory manager to create semantic embeddings for long-term memories
/// and to embed user queries for similarity search.
#[async_trait]
pub trait EmbeddingGenerator: Send + Sync {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>>;
}
