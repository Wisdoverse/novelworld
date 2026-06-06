use async_trait::async_trait;
use uuid::Uuid;
use anyhow::Result;
use crate::domain::entities::narrative_node::{NarrativeNode, WorldState};

#[async_trait]
pub trait NarrativeNodeRepository: Send + Sync {
    async fn save(&self, node: &NarrativeNode) -> Result<()>;
    async fn find_by_chapter(&self, novel_id: Uuid, chapter_number: i32) -> Result<Option<NarrativeNode>>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<NarrativeNode>>;
}

#[async_trait]
pub trait UserChoiceRepository: Send + Sync {
    async fn save_choice(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        node_id: Uuid,
        chapter_number: i32,
        choice_index: i32,
        choice_text: &str,
        consequence: Option<&str>,
    ) -> Result<()>;
    async fn find_user_choice(&self, user_id: Uuid, node_id: Uuid) -> Result<Option<UserChoiceRecord>>;
    async fn find_by_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<Vec<UserChoiceRecord>>;
}

#[async_trait]
pub trait WorldStateRepository: Send + Sync {
    async fn get_or_create(&self, user_id: Uuid, novel_id: Uuid) -> Result<WorldState>;
    async fn update(&self, state: &WorldState) -> Result<()>;
}

/// Lightweight read-only access to chapters and novels from shared DB
#[async_trait]
pub trait ChapterReadRepository: Send + Sync {
    async fn get_chapter_content(&self, novel_id: Uuid, chapter_number: i32) -> Result<Option<String>>;
    async fn get_novel_info(&self, novel_id: Uuid) -> Result<Option<NovelInfo>>;
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct UserChoiceRecord {
    pub id: Uuid,
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub node_id: Uuid,
    pub chapter_number: i32,
    pub choice_index: i32,
    pub choice_text: String,
    pub consequence: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct NovelInfo {
    pub id: Uuid,
    pub title: String,
    pub deviation_mode: String,
    pub world_summary: Option<String>,
}
