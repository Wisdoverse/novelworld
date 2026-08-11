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
    pub reader_identity: Option<String>,
    pub reader_identity_type: String,
    pub reader_character_id: Option<Uuid>,
    pub deviation_mode: String,
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
        max_chapter: i32,
        limit: usize,
    ) -> Result<Vec<ChatMessage>>;
    async fn count(&self, character_id: Uuid, user_id: Uuid, novel_id: Uuid) -> Result<usize>;
    async fn find_by_character_user(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        max_chapter: i32,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChatMessage>>;
}

/// Lightweight character info used by agent-service.
/// Queried from the shared characters table (owned by novel-service).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterInfo {
    pub id: Uuid,
    pub name: String,
    pub novel_id: Uuid,
    pub speaking_style: Option<String>,
    pub first_appearance_chapter: Option<i32>,
}

#[async_trait]
pub trait CharacterInfoRepository: Send + Sync {
    async fn find_by_id(&self, id: Uuid, user_id: Uuid) -> Result<Option<CharacterInfo>>;
}
