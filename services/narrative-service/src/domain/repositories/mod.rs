use crate::domain::entities::{
    game_rules::{ActionCheck, GameRuleTemplate},
    narrative_node::{NarrativeNode, WorldState},
    player_entity::PlayerEntity,
    world_session::{WorldAction, WorldEntryContext, WorldTurnTransition},
};
use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::services::narrative_transition::{CanonContext, NarrativeTransition};

#[derive(Debug, thiserror::Error)]
pub enum GameRuleTemplateRequestError {
    #[error("Game rule template generation is in progress")]
    InProgress { retry_after_seconds: u64 },
    #[error("Game rule template generation budget is exhausted")]
    Exhausted,
    #[error("Novel service is unavailable")]
    Unavailable(#[source] anyhow::Error),
}

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
    async fn create_player_entity(&self, player: &PlayerEntity) -> Result<PlayerEntity>;
    async fn start_open_world(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        context: &WorldEntryContext,
        game_rules: Option<&GameRuleTemplate>,
    ) -> Result<WorldState>;
    async fn update(&self, state: &WorldState) -> Result<()>;
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldTurnClaim {
    pub id: Uuid,
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub request_fingerprint: Vec<u8>,
    pub action: WorldAction,
    #[serde(default)]
    pub resolution: Option<ActionCheck>,
    pub expected_turn_number: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldTurnResult {
    pub turn_id: Uuid,
    pub action: WorldAction,
    #[serde(default)]
    pub resolution: Option<ActionCheck>,
    pub transition: WorldTurnTransition,
    pub world_state: WorldState,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BeginWorldTurn {
    Acquired {
        claim: Box<WorldTurnClaim>,
        attempt: i64,
    },
    Completed(Box<WorldTurnResult>),
    InProgress {
        retry_after_seconds: u64,
    },
    Conflict,
    Stale,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldTurnJournalEntry {
    pub turn_id: Uuid,
    pub turn_number: i64,
    pub action: WorldAction,
    #[serde(default)]
    pub resolution: Option<ActionCheck>,
    pub transition: WorldTurnTransition,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub completed_at: chrono::DateTime<chrono::Utc>,
}

#[async_trait]
pub trait WorldTurnRepository: Send + Sync {
    async fn begin_turn(&self, claim: &WorldTurnClaim) -> Result<BeginWorldTurn>;
    async fn renew_turn(&self, turn_id: Uuid, attempt: i64) -> Result<bool>;
    async fn complete_turn(
        &self,
        claim: &WorldTurnClaim,
        attempt: i64,
        transition: &WorldTurnTransition,
        context: &WorldEntryContext,
    ) -> Result<WorldTurnResult>;
    async fn fail_turn(&self, turn_id: Uuid, attempt: i64, failure_code: &str) -> Result<bool>;
    async fn journal(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        limit: usize,
    ) -> Result<Vec<WorldTurnJournalEntry>>;
}

/// Minimal character identity for journey-memory anchoring.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterBrief {
    pub id: Uuid,
    pub role: String,
    pub first_appearance_chapter: Option<i32>,
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
    async fn list_characters(&self, novel_id: Uuid, user_id: Uuid) -> Result<Vec<CharacterBrief>>;
    async fn get_player_entry_context(
        &self,
        novel_id: Uuid,
        user_id: Uuid,
        checkpoint_chapter: Option<i32>,
        proposed_name: Option<&str>,
    ) -> Result<Option<PlayerEntryContext>>;
    async fn uses_original_player_identity(&self, novel_id: Uuid, user_id: Uuid) -> Result<bool>;
    async fn get_world_entry_context(
        &self,
        novel_id: Uuid,
        checkpoint_chapter: i32,
        user_id: Uuid,
    ) -> Result<Option<WorldEntryContext>>;
    async fn request_game_rule_template(
        &self,
        novel_id: Uuid,
        user_id: Uuid,
    ) -> std::result::Result<GameRuleTemplate, GameRuleTemplateRequestError>;
    async fn get_game_rule_template(
        &self,
        novel_id: Uuid,
        canon_model_version: i32,
        user_id: Uuid,
    ) -> Result<Option<GameRuleTemplate>>;
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
#[serde(deny_unknown_fields)]
pub struct PlayerEntryContext {
    pub checkpoint_chapter: i32,
    pub name_available: bool,
    pub locations: Vec<crate::domain::services::narrative_transition::CanonEntityRef>,
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
