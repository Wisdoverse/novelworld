use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entities::memory::{ChatMessage, Memory, MemoryLayer};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatTurnClaim {
    pub id: Uuid,
    pub user_id: Uuid,
    pub character_id: Uuid,
    pub novel_id: Uuid,
    pub request_fingerprint: Vec<u8>,
    pub chapter_context: i32,
    pub persona_source_chapter_high_water: Option<i32>,
    pub reader_identity: Option<String>,
    pub reader_identity_type: String,
    pub reader_character_id: Option<Uuid>,
    pub deviation_mode: String,
    pub world_revision: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginChatTurn {
    Acquired {
        claim: ChatTurnClaim,
        attempt: i64,
    },
    Completed {
        claim: ChatTurnClaim,
        response: String,
    },
    InProgress {
        retry_after_seconds: u64,
    },
    Conflict,
}

#[async_trait]
pub trait MemoryRepository: Send + Sync {
    /// Atomically reserve a memory id with its durable fact. Returns false
    /// when the id already exists; callers must then validate the existing row
    /// before treating the operation as an idempotent replay.
    async fn insert_if_absent(&self, memory: &Memory) -> Result<bool>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Memory>>;
    async fn save(&self, memory: &Memory) -> Result<()>;
    #[allow(clippy::too_many_arguments)]
    async fn find_by_layer(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        layer: MemoryLayer,
        max_chapter: i32,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Memory>>;
    /// Return two independently bounded permanent-memory buckets: UUIDv5
    /// journey candidates and legacy rows ranked by importance/recency. UUID
    /// version is only a cheap candidate filter; the domain parser remains the
    /// authority for the complete structured-fact contract.
    async fn find_permanent_candidates(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        max_chapter: i32,
        journey_limit: i64,
        legacy_limit: i64,
    ) -> Result<Vec<Memory>>;
    /// Search for memories similar to the given embedding vector using pgvector cosine distance.
    async fn search_similar(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        embedding: &[f32],
        max_chapter: i32,
        limit: usize,
    ) -> Result<Vec<Memory>>;
}

#[async_trait]
pub trait ChatRepository: Send + Sync {
    async fn begin_turn(&self, claim: &ChatTurnClaim) -> Result<BeginChatTurn>;
    async fn renew_turn(&self, turn_id: Uuid, attempt: i64) -> Result<bool>;
    async fn complete_turn(
        &self,
        claim: &ChatTurnClaim,
        attempt: i64,
        user_message: &ChatMessage,
        character_message: &ChatMessage,
    ) -> Result<()>;
    async fn fail_turn(&self, turn_id: Uuid, attempt: i64, failure_code: &str) -> Result<bool>;
    async fn find_recent(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        reader_character_id: Option<Uuid>,
        max_chapter: i32,
        limit: usize,
    ) -> Result<Vec<ChatMessage>>;
    async fn count(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        reader_character_id: Option<Uuid>,
        max_chapter: i32,
    ) -> Result<usize>;
    #[allow(clippy::too_many_arguments)]
    async fn find_by_character_user(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        reader_character_id: Option<Uuid>,
        max_chapter: i32,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChatMessage>>;
}

/// Lightweight character info used by agent-service.
/// Fetched from novel-service through this domain port; agent-service never
/// reads novel-service-owned tables directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInfo {
    pub id: Uuid,
    pub name: String,
    pub novel_id: Uuid,
    pub aliases: Vec<String>,
    pub role: Option<String>,
    pub description: Option<String>,
    pub personality: Option<String>,
    pub background: Option<String>,
    pub speaking_style: Option<String>,
    /// Whole-novel persona is safe only when this server-issued boundary is
    /// within the reading context captured for the chat turn.
    #[serde(default)]
    pub persona_source_chapter_high_water: Option<i32>,
    pub first_appearance_chapter: Option<i32>,
}

#[async_trait]
pub trait CharacterInfoRepository: Send + Sync {
    async fn find_by_id(&self, id: Uuid, user_id: Uuid) -> Result<Option<CharacterInfo>>;
}
