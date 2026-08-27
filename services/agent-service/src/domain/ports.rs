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
    async fn chat_stream(
        &self,
        user_id: Uuid,
        messages: Vec<(String, String)>,
    ) -> Result<ChatStream>;
    async fn chat_messages(&self, user_id: Uuid, messages: Vec<(String, String)>)
        -> Result<String>;
}

/// Port for short-term message caching (Redis or similar).
/// Domain services depend on this trait, not on concrete cache implementations.
#[async_trait]
pub trait MessageCache: Send + Sync {
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
    async fn summarize(&self, user_id: Uuid, system: &str, text: &str) -> Result<String>;
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CharacterContextEnvelope {
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub character_id: Uuid,
    pub branch_context: Option<CharacterBranchContext>,
    pub world_context: Option<CharacterWorldContext>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CharacterBranchContext {
    pub source_chapter_high_water: i32,
    pub events: Vec<CharacterBranchEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CharacterBranchEvent {
    pub chapter_number: i32,
    pub summary: String,
    pub actor_character_ids: Vec<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CharacterWorldContext {
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub character_id: Uuid,
    pub character_alive: bool,
    pub canon_model_version: i32,
    pub checkpoint_chapter: i32,
    /// Highest canonical chapter that may have influenced derived world state.
    /// `None` is accepted only as a rolling-deploy legacy shape; callers must
    /// omit that world context because its spoiler boundary cannot be proven.
    #[serde(default)]
    pub source_chapter_high_water: Option<i32>,
    pub turn_number: i64,
    pub world_time: i64,
    pub player_id: Uuid,
    pub player_name: String,
    pub player_location_id: String,
    pub relationship: Option<WorldRelationship>,
    pub goals: Vec<WorldCharacterGoal>,
    pub perception_of_player: Option<String>,
    pub current_canonical_event: Option<WorldCanonicalEvent>,
    #[serde(default)]
    pub recent_actions: Vec<WorldActionContext>,
    pub recent_player_events: Vec<WorldHistoryItem>,
    pub active_threads: Vec<WorldActiveThread>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldActionContext {
    pub turn_id: Uuid,
    pub turn_number: i64,
    pub action: WorldActionData,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldActionData {
    pub kind: String,
    pub target_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldRelationship {
    pub score: i32,
    pub last_change: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldCharacterGoal {
    pub id: String,
    pub character_id: Uuid,
    pub description: String,
    pub source_chapters: Vec<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldCanonicalEvent {
    pub id: String,
    pub sequence: i32,
    pub summary: String,
    pub character_ids: Vec<Uuid>,
    pub location_ids: Vec<String>,
    pub faction_ids: Vec<String>,
    pub death_character_ids: Vec<Uuid>,
    pub source_chapters: Vec<i32>,
    pub status: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldHistoryItem {
    pub id: String,
    pub turn_id: Uuid,
    pub turn_number: i64,
    pub world_time: i64,
    pub summary: String,
    pub actor_character_ids: Vec<Uuid>,
    pub location_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorldActiveThread {
    pub id: String,
    pub description: String,
    pub origin: String,
}

#[async_trait]
pub trait WorldContextPort: Send + Sync {
    async fn find(
        &self,
        novel_id: Uuid,
        character_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<CharacterContextEnvelope>>;
}
