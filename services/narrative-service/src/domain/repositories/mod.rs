use crate::domain::entities::narrative_node::{NarrativeNode, WorldState};
use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::services::narrative_transition::{CanonContext, NarrativeTransition};

#[async_trait]
pub trait NarrativeNodeRepository: Send + Sync {
    async fn save(&self, node: &NarrativeNode) -> Result<()>;
    async fn find_by_chapter(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
        user_id: Option<Uuid>,
    ) -> Result<Option<NarrativeNode>>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<NarrativeNode>>;
}

#[async_trait]
pub trait UserChoiceRepository: Send + Sync {
    async fn commit_choice(&self, choice: &ChoiceCommit) -> Result<ChoiceCommitResult>;
    async fn find_user_choice(
        &self,
        user_id: Uuid,
        node_id: Uuid,
    ) -> Result<Option<UserChoiceRecord>>;
    async fn find_by_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<Vec<UserChoiceRecord>>;
}

#[async_trait]
pub trait WorldStateRepository: Send + Sync {
    async fn get_or_create(&self, user_id: Uuid, novel_id: Uuid) -> Result<WorldState>;
    async fn update(&self, state: &WorldState) -> Result<()>;
}

/// Read-only access to novel-service data through its HTTP API.
#[async_trait]
pub trait ChapterReadRepository: Send + Sync {
    async fn get_chapter(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
        user_id: Uuid,
    ) -> Result<Option<ChapterInfo>>;
    async fn get_novel_info(&self, novel_id: Uuid, user_id: Uuid) -> Result<Option<NovelInfo>>;
    async fn get_canon_context(
        &self,
        novel_id: Uuid,
        checkpoint_chapter: i32,
        user_id: Uuid,
    ) -> Result<Option<CanonContext>>;
}

#[async_trait]
pub trait PlayerChapterRepository: Send + Sync {
    async fn find(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> Result<Option<PlayerChapter>>;
    async fn save_if_absent(&self, chapter: &PlayerChapter) -> Result<PlayerChapter>;
    async fn find_latest_before(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> Result<Option<PlayerChapter>>;
}

#[derive(Debug, Clone)]
pub struct ChapterInfo {
    pub content: String,
    pub is_key_node: bool,
    pub key_node_description: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct PlayerChapter {
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub chapter_number: i32,
    pub content: String,
    pub origin: PlayerChapterOrigin,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerChapterOrigin {
    Choice,
    Continuation,
}

impl PlayerChapterOrigin {
    pub fn to_str(self) -> &'static str {
        match self {
            Self::Choice => "choice",
            Self::Continuation => "continuation",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(value: &str) -> Option<Self> {
        match value {
            "choice" => Some(Self::Choice),
            "continuation" => Some(Self::Continuation),
            _ => None,
        }
    }
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
    pub consequence: String,
    pub transition: NarrativeTransition,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct ChoiceCommit {
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub node_id: Uuid,
    pub chapter_number: i32,
    pub choice_index: i32,
    pub choice_text: String,
    pub transition: NarrativeTransition,
    pub rewritten_chapter_content: String,
}

#[derive(Debug, Clone)]
pub struct ChoiceCommitResult {
    pub choice: UserChoiceRecord,
    pub world_state: WorldState,
    pub player_chapter_content: String,
}

#[derive(Debug, Clone)]
pub struct NovelInfo {
    pub id: Uuid,
    pub title: String,
    pub deviation_mode: String,
    pub world_summary: Option<String>,
}
