use async_trait::async_trait;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::{novel::Novel, chapter::Chapter, character::Character};

#[async_trait]
pub trait NovelRepository: Send + Sync {
    async fn save(&self, novel: &Novel) -> Result<()>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Novel>>;
    async fn find_by_user(&self, user_id: Uuid) -> Result<Vec<Novel>>;
    async fn update(&self, novel: &Novel) -> Result<()>;
    async fn delete(&self, id: Uuid) -> Result<()>;
}

#[async_trait]
pub trait ChapterRepository: Send + Sync {
    async fn save_batch(&self, chapters: &[Chapter]) -> Result<()>;
    async fn find_by_novel(&self, novel_id: Uuid) -> Result<Vec<Chapter>>;
    async fn find_by_number(&self, novel_id: Uuid, number: i32) -> Result<Option<Chapter>>;
    async fn update(&self, chapter: &Chapter) -> Result<()>;
}

#[async_trait]
pub trait CharacterRepository: Send + Sync {
    async fn save_batch(&self, characters: &[Character]) -> Result<()>;
    async fn find_by_novel(&self, novel_id: Uuid) -> Result<Vec<Character>>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Character>>;
    async fn update(&self, character: &Character) -> Result<()>;
    async fn save_relationship(&self, novel_id: Uuid, from_id: Uuid, to_id: Uuid, rel_type: &str, description: Option<&str>, strength: i32) -> Result<()>;
    async fn find_relationships(&self, novel_id: Uuid) -> Result<Vec<CharacterRelationshipRecord>>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CharacterRelationshipRecord {
    pub id: Uuid,
    pub novel_id: Uuid,
    pub from_character_id: Uuid,
    pub to_character_id: Uuid,
    pub relationship_type: String,
    pub description: Option<String>,
    pub strength: i32,
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingProgressRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub current_chapter: i32,
    pub reader_identity: Option<String>,
    pub reader_identity_type: String,
    pub reader_character_id: Option<Uuid>,
    pub deviation_mode: String,
    pub last_read_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[async_trait]
pub trait ReadingProgressRepository: Send + Sync {
    async fn get_or_create(&self, user_id: Uuid, novel_id: Uuid) -> Result<ReadingProgressRecord>;
    async fn update_chapter(&self, user_id: Uuid, novel_id: Uuid, chapter: i32) -> Result<()>;
    async fn set_identity(&self, user_id: Uuid, novel_id: Uuid, identity_type: &str, identity_name: Option<&str>, character_id: Option<Uuid>) -> Result<()>;
}
