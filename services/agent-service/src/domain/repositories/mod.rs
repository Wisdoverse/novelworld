use async_trait::async_trait;
use uuid::Uuid;
use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::domain::entities::memory::{Memory, MemoryLayer, ChatMessage};

#[async_trait]
pub trait MemoryRepository: Send + Sync {
    async fn save(&self, memory: &Memory) -> Result<()>;
    async fn find_by_layer(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        layer: MemoryLayer,
    ) -> Result<Vec<Memory>>;
}

#[async_trait]
pub trait ChatRepository: Send + Sync {
    async fn save(&self, msg: &ChatMessage) -> Result<()>;
    async fn find_recent(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ChatMessage>>;
    async fn count(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<usize>;
    async fn find_by_character_user(
        &self,
        character_id: Uuid,
        user_id: Uuid,
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
    pub system_prompt: Option<String>,
}

#[async_trait]
pub trait CharacterInfoRepository: Send + Sync {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<CharacterInfo>>;
}
