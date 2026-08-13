use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use uuid::Uuid;

use crate::domain::entities::memory::ChatMessage;

#[derive(Debug)]
pub struct AccountExportRecord {
    pub kind: String,
    pub data: serde_json::Value,
}

pub type AccountExportStream =
    Pin<Box<dyn futures::Stream<Item = Result<AccountExportRecord>> + Send>>;

pub trait AccountExportPort: Send + Sync {
    fn export_user(&self, user_id: Uuid) -> AccountExportStream;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatCompletionEvent {
    Delta(String),
    Finished,
}

pub type ChatStream = Pin<Box<dyn futures::Stream<Item = Result<ChatCompletionEvent>> + Send>>;

#[async_trait]
pub trait ChatCompletion: Send + Sync {
    async fn chat_stream(&self, messages: Vec<(String, String)>) -> Result<ChatStream>;
    async fn chat_messages(&self, messages: Vec<(String, String)>) -> Result<String>;
}

/// Port for short-term message caching (Redis or similar).
/// Domain services depend on this trait, not on concrete cache implementations.
#[async_trait]
pub trait MessageCache: Send + Sync {
    async fn get_recent_messages(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        max_chapter: i32,
        limit: usize,
    ) -> Result<Vec<ChatMessage>>;

    async fn push_turn(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        user_message: &ChatMessage,
        character_message: &ChatMessage,
    ) -> Result<bool>;

    async fn clear(&self, character_id: Uuid, user_id: Uuid) -> Result<()>;

    async fn clear_user(&self, user_id: Uuid) -> Result<()>;

    async fn clear_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()>;

    async fn allow_user(&self, user_id: Uuid) -> Result<()>;

    async fn allow_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()>;
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

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
    async fn is_ready(&self) -> bool;
}

#[derive(Debug, Clone, PartialEq)]
pub struct ReadingContext {
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub current_chapter: i32,
    pub reader_identity: Option<String>,
    pub reader_identity_type: String,
    pub reader_character_id: Option<Uuid>,
    pub deviation_mode: String,
}

#[async_trait]
pub trait ReadingContextPort: Send + Sync {
    async fn find(&self, novel_id: Uuid, user_id: Uuid) -> Result<Option<ReadingContext>>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoreExcerpt {
    pub chapter_number: i32,
    pub title: Option<String>,
    pub content: String,
    pub score: f32,
}

#[async_trait]
pub trait LoreContextPort: Send + Sync {
    async fn search(
        &self,
        novel_id: Uuid,
        user_id: Uuid,
        max_chapter: i32,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LoreExcerpt>>;
}
