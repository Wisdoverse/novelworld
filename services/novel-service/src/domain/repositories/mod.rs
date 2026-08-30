use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::entities::{
    canon_story_model::CanonStoryModel, chapter::Chapter, character::Character,
    game_rule_template::GameRuleTemplate, novel::Novel,
};
use crate::domain::value_objects::{DeviationMode, ImportStage};

/// `import-provider-budget-v1` (docs/IMPORT_BUDGET.md): a job must not be
/// claimed more than three times. Enforced by the persistence adapter at the
/// claim boundary.
pub const MAX_IMPORT_ATTEMPTS: i64 = 3;
/// Public, actionable guidance stored on the Novel when the attempt ceiling
/// is reached; the retry endpoint surfaces it without an application provider
/// dispatch.
pub const IMPORT_BUDGET_EXHAUSTED_MESSAGE: &str =
    "Import provider budget exhausted; re-upload the source";
// ponytail: terminal three-attempt budget; add an operator retry workflow only
// when real support cases justify more provider-facing state and UI.
pub const MAX_GAME_RULE_GENERATION_ATTEMPTS: i64 = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportClaim {
    pub novel_id: Uuid,
    pub user_id: Uuid,
    pub stage: ImportStage,
    pub attempt: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoverableImport {
    pub novel_id: Uuid,
    pub user_id: Uuid,
}

#[async_trait]
pub trait NovelRepository: Send + Sync {
    async fn create_import(&self, novel: &Novel, chapters: &[Chapter]) -> Result<()>;
    /// Atomically accept multiple independent Novel aggregates and their
    /// durable import jobs. An empty chapter list denotes a retained-source
    /// import; every other item must contain importable chapters.
    async fn create_import_batch(&self, imports: &[(Novel, Vec<Chapter>)]) -> Result<()>;
    /// Commit a Novel plus a stage-`source` job without chapters. Only used
    /// when source retention is enabled; the claimed worker rebuilds chapters
    /// from the retained object before any provider work.
    async fn create_source_import(&self, novel: &Novel) -> Result<()>;
    async fn claim_import(&self, novel_id: Uuid, user_id: Uuid) -> Result<Option<ImportClaim>>;
    async fn recoverable_imports(&self, limit: i64) -> Result<Vec<RecoverableImport>>;
    async fn renew_import(&self, novel_id: Uuid, attempt: i64) -> Result<bool>;
    /// Atomically replace the novel's chapters before enrichment, fenced by
    /// `(novel_id, attempt)`. A `source` job advances to `chapters`; an
    /// existing `chapters` job stays there while boundary repair is applied.
    async fn replace_import_chapters(
        &self,
        novel_id: Uuid,
        attempt: i64,
        chapters: &[Chapter],
    ) -> Result<bool>;
    async fn record_import_enrichment(
        &self,
        novel_id: Uuid,
        attempt: i64,
        total_chapters: i32,
        world_summary: &str,
        genre: &str,
    ) -> Result<bool>;
    async fn complete_import(&self, novel_id: Uuid, attempt: i64) -> Result<bool>;
    async fn fail_import(
        &self,
        novel_id: Uuid,
        attempt: i64,
        failure_code: &str,
        public_error: &str,
    ) -> Result<bool>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Novel>>;
    async fn find_for_user(&self, user_id: Uuid, novel_id: Uuid) -> Result<Option<Novel>>;
    async fn find_by_user(&self, user_id: Uuid) -> Result<Vec<Novel>>;
    async fn find_available_to_user(&self, user_id: Uuid) -> Result<Vec<Novel>>;
    async fn attach_to_user(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        deviation_mode: DeviationMode,
    ) -> Result<bool>;
    async fn detach_from_user(&self, user_id: Uuid, novel_id: Uuid) -> Result<bool>;
}

#[derive(Debug, Clone)]
pub struct PendingSourceFileDeletion {
    pub object_key: String,
    pub attempts: i32,
    /// Opaque fencing token for this cleanup claim. Completion and retry from
    /// an older worker must not consume a requeued or newly claimed intent.
    pub claim_token: String,
}

#[async_trait]
pub trait SourceFileDeletionRepository: Send + Sync {
    /// Reserve a newly uploaded object until the accepting Novel transaction
    /// consumes the reservation. Cleanup may claim it only after `not_before`.
    async fn reserve_upload(&self, object_key: &str, not_before: DateTime<Utc>) -> Result<()>;
    /// Restore a due deletion intent after an upload completed but the Novel
    /// transaction could not consume its reservation. This must replace any
    /// stale cleanup claim so a late upload cannot become orphaned.
    async fn enqueue_cleanup(&self, object_key: &str, not_before: DateTime<Utc>) -> Result<()>;
    async fn due(&self, limit: i64) -> Result<Vec<PendingSourceFileDeletion>>;
    async fn complete(&self, object_key: &str, claim_token: &str) -> Result<()>;
    async fn retry(
        &self,
        object_key: &str,
        claim_token: &str,
        error: &str,
        not_before: DateTime<Utc>,
    ) -> Result<()>;
}

pub struct CanonExtractionCheckpoint<'a> {
    pub novel_id: Uuid,
    pub model_version: i32,
    pub prompt_version: &'a str,
    pub chapter_number: i32,
    pub chunk_index: i32,
    pub is_final: bool,
    pub source_content: &'a str,
    pub extraction_json: &'a str,
}

#[derive(Debug, Clone, PartialEq)]
pub enum BeginGameRuleGeneration {
    Acquired { attempt: i64 },
    Ready(GameRuleTemplate),
    InProgress { retry_after_seconds: u64 },
    Exhausted,
}

#[async_trait]
pub trait CanonStoryModelRepository: Send + Sync {
    async fn find_import_checkpoint(
        &self,
        novel_id: Uuid,
        model_version: i32,
        prompt_version: &str,
        chapter_number: i32,
        chunk_index: i32,
        source_content: &str,
    ) -> Result<Option<String>>;
    async fn save_import_checkpoint(
        &self,
        checkpoint: CanonExtractionCheckpoint<'_>,
        attempt: i64,
    ) -> Result<bool>;
    async fn insert_import(&self, model: &CanonStoryModel, attempt: i64) -> Result<bool>;
    async fn find_version(
        &self,
        novel_id: Uuid,
        model_version: i32,
    ) -> Result<Option<CanonStoryModel>>;
    async fn find_latest(&self, novel_id: Uuid) -> Result<Option<CanonStoryModel>>;
    async fn begin_game_rule_generation(
        &self,
        novel_id: Uuid,
        canon_model_version: i32,
    ) -> Result<BeginGameRuleGeneration>;
    async fn renew_game_rule_generation(
        &self,
        novel_id: Uuid,
        canon_model_version: i32,
        attempt: i64,
    ) -> Result<bool>;
    async fn complete_game_rule_generation(
        &self,
        template: &GameRuleTemplate,
        attempt: i64,
    ) -> Result<bool>;
    async fn fail_game_rule_generation(
        &self,
        novel_id: Uuid,
        canon_model_version: i32,
        attempt: i64,
        failure_code: &str,
    ) -> Result<bool>;
    async fn find_game_rule_template(
        &self,
        novel_id: Uuid,
        canon_model_version: i32,
    ) -> Result<Option<GameRuleTemplate>>;
}

#[async_trait]
pub trait ChapterRepository: Send + Sync {
    async fn replace_import_nodes(
        &self,
        novel_id: Uuid,
        attempt: i64,
        nodes: &[(i32, String)],
    ) -> Result<bool>;
    async fn find_by_novel(&self, novel_id: Uuid) -> Result<Vec<Chapter>>;
    async fn find_by_number(&self, novel_id: Uuid, number: i32) -> Result<Option<Chapter>>;
    async fn search_lore(
        &self,
        novel_id: Uuid,
        max_chapter: i32,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LoreExcerpt>>;
}

#[derive(Debug, Clone, Copy)]
pub struct ChapterTranslationKey<'a> {
    pub chapter_id: Uuid,
    pub source_sha256: &'a [u8],
    pub profile: &'a str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BeginChapterTranslation {
    Acquired { attempt: i64 },
    Ready(String),
    InProgress { retry_after_seconds: u64 },
}

#[async_trait]
pub trait ChapterTranslationRepository: Send + Sync {
    async fn find_ready(&self, key: ChapterTranslationKey<'_>) -> Result<Option<String>>;
    async fn begin(&self, key: ChapterTranslationKey<'_>) -> Result<BeginChapterTranslation>;
    async fn complete(
        &self,
        key: ChapterTranslationKey<'_>,
        attempt: i64,
        translated_content: &str,
    ) -> Result<bool>;
    async fn fail(
        &self,
        key: ChapterTranslationKey<'_>,
        attempt: i64,
        failure_code: &str,
    ) -> Result<bool>;
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LoreExcerpt {
    pub chapter_number: i32,
    pub title: Option<String>,
    pub content: String,
    pub score: f32,
}

#[async_trait]
pub trait CharacterRepository: Send + Sync {
    async fn replace_import(
        &self,
        novel_id: Uuid,
        attempt: i64,
        characters: &[Character],
        relationships: &[CharacterRelationshipRecord],
    ) -> Result<bool>;
    async fn find_by_novel(&self, novel_id: Uuid) -> Result<Vec<Character>>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<Character>>;
    async fn set_avatar(&self, character_id: Uuid, avatar_url: &str) -> Result<()>;
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
    async fn get_or_create(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        deviation_mode: &str,
    ) -> Result<ReadingProgressRecord>;
    async fn update_chapter(&self, user_id: Uuid, novel_id: Uuid, chapter: i32) -> Result<()>;
    async fn set_identity(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        identity_type: &str,
        identity_name: Option<&str>,
        character_id: Option<Uuid>,
    ) -> Result<()>;
}
