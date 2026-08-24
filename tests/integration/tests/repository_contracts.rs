use agent_service::domain::{
    entities::memory::{ChatMessage, Memory, MemoryLayer},
    repositories::{BeginChatTurn, ChatRepository, ChatTurnClaim, MemoryRepository},
};
use agent_service::infrastructure::persistence::{
    pg_chat_repo::PgChatRepository, pg_memory_repo::PgMemoryRepository,
};
use narrative_service::domain::{
    entities::{
        game_rules::ActionCheck,
        narrative_node::{NarrativeChoice, NarrativeNode, WorldState, WorldStateError},
        player_entity::PlayerEntity,
        world_session::{
            CanonicalEventChange, CanonicalEventStatus, CharacterGoalRef, FactionStandingChange,
            ScheduledCanonEvent, WorldAction, WorldActionKind, WorldCharacterRef, WorldEntityRef,
            WorldEntryContext, WorldRuleRef, WorldTurnTransition, WORLD_TURN_PROMPT_VERSION,
            WORLD_TURN_SCHEMA_VERSION,
        },
    },
    repositories::{
        BeginWorldTurn, ChoiceCommit, MemoryProjectionStatus, NarrativeNodeRepository,
        UserChoiceRepository, WorldStateRepository, WorldTurnClaim, WorldTurnRepository,
    },
    services::narrative_transition::{
        NarrativeTransition, RelationshipChange, TransitionEvent, TRANSITION_PROMPT_VERSION,
        TRANSITION_SCHEMA_VERSION,
    },
};
use narrative_service::infrastructure::persistence::pg_narrative_repo::{
    PgNarrativeNodeRepository, PgUserChoiceRepository,
};
use narrative_service::infrastructure::persistence::pg_world_state_repo::PgWorldStateRepository;
use narrative_service::infrastructure::persistence::pg_world_turn_repo::PgWorldTurnRepository;
use novel_service::application::{commands::ImportNovelCommand, handlers::NovelCommandHandler};
use novel_service::domain::{
    entities::canon_story_model::{
        CanonEndingSnapshot, CanonEvent, CanonStoryContent, CanonStoryModel, SourceCitation,
        SourceEvidence, StoryArc, CANON_STORY_SCHEMA_VERSION,
    },
    entities::chapter::Chapter,
    entities::character::Character,
    entities::game_rule_template::{
        GameActionKind, GameActionRule, GameAttribute, GameRuleTemplate,
    },
    entities::novel::Novel,
    ports::{ImagePort, LlmPort, NovelLlmTask, PrivacyCleanupPort, SourceFileStorage},
    repositories::{
        BeginChapterTranslation, BeginGameRuleGeneration, CanonExtractionCheckpoint,
        CanonStoryModelRepository, ChapterRepository, ChapterTranslationKey,
        ChapterTranslationRepository, CharacterRelationshipRecord, CharacterRepository,
        NovelRepository, ReadingProgressRepository, SourceFileDeletionRepository,
        IMPORT_BUDGET_EXHAUSTED_MESSAGE, MAX_IMPORT_ATTEMPTS,
    },
    value_objects::{CharacterRole, DeviationMode, ImportStage},
};
use novel_service::infrastructure::document::EbookTextExtractor;
use novel_service::infrastructure::persistence::{
    canon_story_model_pg_repo::PgCanonStoryModelRepository, chapter_pg_repo::ChapterPgRepository,
    chapter_translation_pg_repo::PgChapterTranslationRepository,
    character_pg_repo::CharacterPgRepository, novel_pg_repo::NovelPgRepository,
    pg_progress_repo::PgReadingProgressRepository,
    source_file_deletion_pg_repo::PgSourceFileDeletionRepository,
};
use sqlx::{postgres::PgPoolOptions, PgPool};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://test:test@localhost:25432/novelworld_test".into())
}

struct BlockingImportLlm;

#[async_trait::async_trait]
impl LlmPort for BlockingImportLlm {
    async fn chat_json(&self, _task: NovelLlmTask, _prompt: &str) -> anyhow::Result<String> {
        futures::future::pending().await
    }
}

struct UnusedImage;

#[async_trait::async_trait]
impl ImagePort for UnusedImage {
    async fn generate(&self, _prompt: &str) -> anyhow::Result<String> {
        anyhow::bail!("image generation must not run before import completion")
    }
}

struct NoopNovelPrivacy;

#[async_trait::async_trait]
impl PrivacyCleanupPort for NoopNovelPrivacy {
    async fn clear_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }

    async fn allow_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
}

fn blocking_import_handler(
    pool: &PgPool,
    source_storage: Option<Arc<dyn SourceFileStorage>>,
) -> Arc<NovelCommandHandler> {
    Arc::new(NovelCommandHandler {
        novel_repo: Arc::new(NovelPgRepository::new(pool.clone())),
        chapter_repo: Arc::new(ChapterPgRepository::new(pool.clone())),
        character_repo: Arc::new(CharacterPgRepository::new(pool.clone())),
        canon_repo: Arc::new(PgCanonStoryModelRepository::new(pool.clone())),
        llm: Arc::new(BlockingImportLlm),
        image_client: Arc::new(UnusedImage),
        privacy_cleanup: Arc::new(NoopNovelPrivacy),
        source_storage,
        source_deletions: Arc::new(PgSourceFileDeletionRepository::new(pool.clone())),
        document_extractor: Arc::new(EbookTextExtractor),
        import_permits: Arc::new(Semaphore::new(1)),
        active_import_users: Arc::new(Mutex::new(HashSet::new())),
    })
}

fn transition(chapter: i32, rendered_narrative: impl Into<String>) -> NarrativeTransition {
    NarrativeTransition {
        schema_version: TRANSITION_SCHEMA_VERSION,
        prompt_version: TRANSITION_PROMPT_VERSION.into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: chapter,
        rendered_narrative: rendered_narrative.into(),
        events: vec![TransitionEvent {
            summary: "玩家的选择改变了局势".into(),
            actor_character_ids: vec![],
            location_id: None,
        }],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
    }
}

#[tokio::test]
async fn accepted_import_already_has_durable_chapters_and_claim() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("accepted-import-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();

    let handler = blocking_import_handler(&pool, None);
    let novel_id = handler
        .handle_import(ImportNovelCommand {
            user_id,
            title: "Accepted import".into(),
            author: None,
            raw_content: Some("A durable source paragraph. ".repeat(20)),
            source_bytes: None,
            deviation_mode: None,
        })
        .await
        .unwrap();

    let state: (String, String, String, i64, bool, i64) = sqlx::query_as(
        "SELECT novel.status::text, job.stage, job.status, job.attempt, \
                job.lease_expires_at IS NOT NULL, \
                (SELECT COUNT(*) FROM chapters WHERE novel_id = novel.id) \
         FROM novels AS novel \
         JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
         WHERE novel.id = $1",
    )
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        state,
        (
            "parsing".into(),
            "chapters".into(),
            "in_progress".into(),
            1,
            true,
            1,
        )
    );
}

#[tokio::test]
async fn shared_novel_has_private_shelves_progress_and_worlds() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();
    let uploader = insert_test_user(&pool, "shared-uploader").await;
    let reader = insert_test_user(&pool, "shared-reader").await;
    let mut novel = Novel::create(uploader, "One canonical novel".into(), None);
    novel.set_deviation_mode(DeviationMode::Creative);
    let chapter = Chapter::new(
        novel.id,
        1,
        Some("Shared chapter".into()),
        "The same canonical chapter is parsed exactly once. ".repeat(4),
    );
    let repo = NovelPgRepository::new(pool.clone());
    repo.create_import(&novel, &[chapter]).await.unwrap();
    sqlx::query(
        "UPDATE novels SET status = 'ready'::novel_status, total_chapters = 1 WHERE id = $1",
    )
    .bind(novel.id)
    .execute(&pool)
    .await
    .unwrap();

    assert!(repo
        .find_for_user(uploader, novel.id)
        .await
        .unwrap()
        .is_some());
    assert!(repo
        .find_available_to_user(reader)
        .await
        .unwrap()
        .iter()
        .any(|candidate| candidate.id == novel.id));
    assert!(repo
        .attach_to_user(reader, novel.id, DeviationMode::Remix)
        .await
        .unwrap());
    assert!(!repo
        .find_available_to_user(reader)
        .await
        .unwrap()
        .iter()
        .any(|candidate| candidate.id == novel.id));

    let reader_novel = repo.find_for_user(reader, novel.id).await.unwrap().unwrap();
    assert_eq!(reader_novel.deviation_mode, DeviationMode::Remix);
    let modes: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT user_id, deviation_mode::text FROM reading_progress \
         WHERE novel_id = $1 ORDER BY user_id",
    )
    .bind(novel.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(modes.contains(&(uploader, "creative".into())));
    assert!(modes.contains(&(reader, "remix".into())));

    for (user_id, marker) in [(uploader, "uploader-world"), (reader, "reader-world")] {
        sqlx::query(
            "INSERT INTO world_states (id, user_id, novel_id, state) \
             VALUES ($1, $2, $3, jsonb_build_object('marker', $4::text))",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(novel.id)
        .bind(marker)
        .execute(&pool)
        .await
        .unwrap();
    }
    let worlds: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT user_id, state->>'marker' FROM world_states WHERE novel_id = $1 ORDER BY user_id",
    )
    .bind(novel.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(worlds.contains(&(uploader, "uploader-world".into())));
    assert!(worlds.contains(&(reader, "reader-world".into())));

    assert!(repo.detach_from_user(reader, novel.id).await.unwrap());
    assert!(repo
        .find_for_user(reader, novel.id)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state->>'marker' FROM world_states WHERE user_id = $1 AND novel_id = $2",
        )
        .bind(reader)
        .bind(novel.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        "reader-world"
    );
    assert!(repo
        .attach_to_user(reader, novel.id, DeviationMode::Canon)
        .await
        .unwrap());
    assert_eq!(
        repo.find_for_user(reader, novel.id)
            .await
            .unwrap()
            .unwrap()
            .deviation_mode,
        DeviationMode::Canon
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(uploader)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.find_by_id(novel.id).await.unwrap().is_some());
    assert!(repo
        .find_for_user(reader, novel.id)
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM world_states WHERE novel_id = $1")
            .bind(novel.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(reader)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(novel.id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn startup_recovery_claims_a_pending_import() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("startup-recovery-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    let novel = Novel::create(user_id, "Startup recovery".into(), None);
    let chapter = Chapter::new(
        novel.id,
        1,
        None,
        "A durable source chapter waiting for startup recovery.".repeat(4),
    );
    NovelPgRepository::new(pool.clone())
        .create_import(&novel, &[chapter])
        .await
        .unwrap();
    sqlx::query(
        "UPDATE novel_import_jobs \
         SET created_at = NOW() - INTERVAL '100 years', \
             updated_at = NOW() - INTERVAL '100 years' \
         WHERE novel_id = $1",
    )
    .bind(novel.id)
    .execute(&pool)
    .await
    .unwrap();

    let recovery = blocking_import_handler(&pool, None).spawn_import_recovery();
    let state = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let state = sqlx::query_as::<_, (String, i64, bool)>(
                "SELECT status, attempt, lease_expires_at IS NOT NULL \
                 FROM novel_import_jobs WHERE novel_id = $1",
            )
            .bind(novel.id)
            .fetch_one(&pool)
            .await
            .unwrap();
            if state.0 == "in_progress" {
                break state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("startup recovery must claim the oldest pending import");
    recovery.abort();
    assert_eq!(state, ("in_progress".into(), 1, true));
}

#[tokio::test]
async fn durable_import_claim_is_recoverable_and_attempt_fenced() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("durable-import-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();

    let novel = Novel::create(user_id, "Durable import".into(), None);
    let chapters = vec![Chapter::new(
        novel.id,
        1,
        Some("Chapter 1".into()),
        "A source-backed chapter long enough for durable import testing.".repeat(4),
    )];
    let repo = NovelPgRepository::new(pool.clone());
    let invalid_chapter = Chapter::new(novel.id, 2, None, "out of order".repeat(20));
    assert!(repo
        .create_import(&novel, &[invalid_chapter])
        .await
        .is_err());
    assert!(repo.find_by_id(novel.id).await.unwrap().is_none());
    repo.create_import(&novel, &chapters).await.unwrap();

    let (status, stage, attempt, lease): (
        String,
        String,
        i64,
        Option<chrono::DateTime<chrono::Utc>>,
    ) = sqlx::query_as(
        "SELECT novel.status::text, job.stage, job.attempt, job.lease_expires_at \
         FROM novels AS novel \
         JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
         WHERE novel.id = $1",
    )
    .bind(novel.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        (status.as_str(), stage.as_str(), attempt),
        ("pending", "chapters", 0)
    );
    assert!(lease.is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM chapters WHERE novel_id = $1")
            .bind(novel.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );

    let first = repo
        .claim_import(novel.id, user_id)
        .await
        .unwrap()
        .expect("pending import must be claimable");
    assert_eq!(first.stage, ImportStage::Chapters);
    assert_eq!(first.attempt, 1);
    assert!(repo
        .claim_import(novel.id, user_id)
        .await
        .unwrap()
        .is_none());
    assert!(repo.renew_import(novel.id, first.attempt).await.unwrap());

    sqlx::query(
        "UPDATE novel_import_jobs \
         SET lease_expires_at = NOW() - INTERVAL '1 second' \
         WHERE novel_id = $1",
    )
    .bind(novel.id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(repo
        .recoverable_imports(100)
        .await
        .unwrap()
        .iter()
        .any(|candidate| candidate.novel_id == novel.id && candidate.user_id == user_id));

    let second = repo
        .claim_import(novel.id, user_id)
        .await
        .unwrap()
        .expect("expired import must be reclaimable");
    assert_eq!(second.attempt, 2);
    assert!(!repo.renew_import(novel.id, first.attempt).await.unwrap());
    assert!(repo
        .complete_import(novel.id, second.attempt)
        .await
        .is_err());
    sqlx::query("UPDATE novel_import_jobs SET stage = 'source' WHERE novel_id = $1")
        .bind(novel.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo
        .record_import_enrichment(novel.id, second.attempt, 1, "world", "genre")
        .await
        .is_err());
    sqlx::query("UPDATE novel_import_jobs SET stage = 'chapters' WHERE novel_id = $1")
        .bind(novel.id)
        .execute(&pool)
        .await
        .unwrap();
    let chapter_repo = ChapterPgRepository::new(pool.clone());
    assert!(!chapter_repo
        .replace_import_nodes(
            novel.id,
            first.attempt,
            &[(1, "过期任务不能覆盖节点。".into())],
        )
        .await
        .unwrap());
    assert!(chapter_repo
        .replace_import_nodes(
            novel.id,
            second.attempt,
            &[(1, "当前任务提交可信节点。".into())],
        )
        .await
        .unwrap());
    assert_eq!(
        sqlx::query_scalar::<_, Option<String>>(
            "SELECT key_node_description FROM chapters \
             WHERE novel_id = $1 AND chapter_number = 1",
        )
        .bind(novel.id)
        .fetch_one(&pool)
        .await
        .unwrap()
        .as_deref(),
        Some("当前任务提交可信节点。")
    );
    assert!(!repo
        .record_import_enrichment(novel.id, first.attempt, 1, "world", "genre")
        .await
        .unwrap());
    assert!(!repo
        .fail_import(novel.id, first.attempt, "stale", "stale worker")
        .await
        .unwrap());

    let character_repo = CharacterPgRepository::new(pool.clone());
    let character = Character::new(novel.id, "Durable hero".into(), CharacterRole::Protagonist);
    assert!(character_repo
        .replace_import(novel.id, second.attempt, &[character], &[])
        .await
        .unwrap());
    assert!(repo
        .record_import_enrichment(novel.id, second.attempt, 2, "world", "genre")
        .await
        .is_err());
    assert!(repo
        .record_import_enrichment(novel.id, second.attempt, 1, "world", "genre")
        .await
        .unwrap());
    assert!(repo
        .complete_import(novel.id, second.attempt)
        .await
        .is_err());
    sqlx::query(
        "INSERT INTO canon_story_models (novel_id, model_version, schema_version, prompt_version, content) \
         VALUES ($1, 1, 1, 'durable-import-test-v1', '{}'::jsonb)",
    )
    .bind(novel.id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(repo
        .complete_import(novel.id, second.attempt)
        .await
        .unwrap());

    let (status, stage, lease): (String, String, Option<chrono::DateTime<chrono::Utc>>) =
        sqlx::query_as(
            "SELECT novel.status::text, job.stage, job.lease_expires_at \
             FROM novels AS novel \
             JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
             WHERE novel.id = $1",
        )
        .bind(novel.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((status.as_str(), stage.as_str()), ("ready", "completed"));
    assert!(lease.is_none());
    assert!(repo
        .claim_import(novel.id, user_id)
        .await
        .unwrap()
        .is_none());
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM novel_import_jobs WHERE novel_id = $1)",
    )
    .bind(novel.id)
    .fetch_one(&pool)
    .await
    .unwrap());
}

/// Seed a legacy-shaped novel whose persisted chapters skip chapter 2.
async fn seed_gapped_novel(
    pool: &PgPool,
    novel_status: &str,
    job: Option<(&str, &str)>,
) -> (Uuid, Uuid) {
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("gapped-import-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, world_summary, genre, total_chapters, status) \
         VALUES ($1, $2, 'Gapped legacy novel', 'world', 'genre', 2, $3::novel_status)",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind(novel_status)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chapters (novel_id, chapter_number, content) \
         VALUES ($1, 1, 'A durable first chapter.'), ($1, 3, 'A durable third chapter.')",
    )
    .bind(novel_id)
    .execute(pool)
    .await
    .unwrap();
    if let Some((stage, status)) = job {
        // Recovery orders candidates by age, so the pending job is claimed
        // before any leftover import of a neighbouring test.
        sqlx::query(
            "INSERT INTO novel_import_jobs \
                 (novel_id, stage, status, attempt, lease_expires_at, failure_code, created_at) \
             VALUES ($1, $2, $3, \
                     CASE WHEN $3 = 'in_progress' THEN 1 ELSE 0 END, \
                     CASE WHEN $3 = 'in_progress' THEN NOW() + INTERVAL '2 minutes' END, \
                     CASE WHEN $3 = 'failed' THEN 'legacy_chapters_invalid' END, \
                     NOW() - INTERVAL '500 years')",
        )
        .bind(novel_id)
        .bind(stage)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    }
    (user_id, novel_id)
}

#[tokio::test]
async fn resumed_import_with_gapped_chapters_fails_with_reupload_guidance() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id) =
        seed_gapped_novel(&pool, "error", Some(("chapters", "pending"))).await;

    let handler = blocking_import_handler(&pool, None);
    assert_eq!(
        handler
            .retry_import(user_id, novel_id)
            .await
            .unwrap_err()
            .to_string(),
        "No parsed chapters are available; re-upload the source"
    );

    let recovery = handler.spawn_import_recovery();
    let failure = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let state = sqlx::query_as::<_, (String, Option<String>, String, Option<String>)>(
                "SELECT job.status, job.failure_code, novel.status::text, novel.parse_error \
                 FROM novel_import_jobs AS job \
                 JOIN novels AS novel ON novel.id = job.novel_id \
                 WHERE job.novel_id = $1",
            )
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
            if state.0 == "failed" {
                break state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("a gapped import must fail instead of waiting on a provider");
    recovery.abort();
    assert_eq!(
        failure,
        (
            "failed".into(),
            Some("source_unavailable".into()),
            "error".into(),
            Some("No parsed chapters are available; re-upload the source".into()),
        )
    );
}

fn retained_novel_text() -> String {
    format!(
        "第一章 山门\n{}\n第二章 远行\n{}\n",
        "风雨欲来，少年站在山门之前，望着云雾深处的石阶。".repeat(8),
        "他背起行囊，踏上了通往北方冰原的漫漫长路。".repeat(8),
    )
}

struct FakeSourceStorage {
    bytes: Mutex<Option<bytes::Bytes>>,
    fail: bool,
    gate: Option<Arc<tokio::sync::Semaphore>>,
}

impl FakeSourceStorage {
    fn with_bytes(bytes: Option<bytes::Bytes>) -> Self {
        Self {
            bytes: Mutex::new(bytes),
            fail: false,
            gate: None,
        }
    }

    /// The `get` call parks until the test adds a permit; permits persist even
    /// when granted before the worker arrives, so this is race-free.
    fn gated(bytes: bytes::Bytes) -> (Arc<Self>, Arc<tokio::sync::Semaphore>) {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        (
            Arc::new(Self {
                bytes: Mutex::new(Some(bytes)),
                fail: false,
                gate: Some(gate.clone()),
            }),
            gate,
        )
    }

    fn failing() -> Self {
        Self {
            bytes: Mutex::new(None),
            fail: true,
            gate: None,
        }
    }
}

#[async_trait::async_trait]
impl SourceFileStorage for FakeSourceStorage {
    async fn put(&self, _key: &str, _data: bytes::Bytes) -> anyhow::Result<()> {
        Ok(())
    }

    async fn get(&self, _key: &str) -> anyhow::Result<Option<bytes::Bytes>> {
        if self.fail {
            anyhow::bail!("simulated object storage read failure");
        }
        if let Some(gate) = &self.gate {
            gate.acquire().await.unwrap().forget();
        }
        Ok(self.bytes.lock().unwrap().clone())
    }

    async fn delete(&self, _key: &str) -> anyhow::Result<()> {
        Ok(())
    }
}

async fn insert_test_user(pool: &PgPool, label: &str) -> Uuid {
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("{label}-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(pool)
        .await
        .unwrap();
    user_id
}

#[tokio::test]
async fn chapter_translation_cache_is_durable_single_owner_fenced_and_cascades() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "chapter-translation-cache").await;
    let novel_id = Uuid::new_v4();
    let chapter_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status) \
         VALUES ($1, $2, 'Translation cache contract', 1, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chapters (id, novel_id, chapter_number, content) \
         VALUES ($1, $2, 1, 'English source')",
    )
    .bind(chapter_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();

    let repository = PgChapterTranslationRepository::new(pool.clone());
    let second_instance = PgChapterTranslationRepository::new(pool.clone());
    let source_hash = vec![7_u8; 32];
    let key = ChapterTranslationKey {
        chapter_id,
        source_sha256: &source_hash,
        profile: "zh-cn-v1",
    };
    let (first, second) = tokio::join!(repository.begin(key), second_instance.begin(key));
    let outcomes = [first.unwrap(), second.unwrap()];
    let attempt = outcomes
        .iter()
        .find_map(|outcome| match outcome {
            BeginChapterTranslation::Acquired { attempt } => Some(*attempt),
            _ => None,
        })
        .expect("one concurrent request must own the translation lease");
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginChapterTranslation::Acquired { .. }))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginChapterTranslation::InProgress { .. }))
            .count(),
        1
    );
    assert!(!repository
        .complete(key, attempt + 1, "错误的旧 owner")
        .await
        .unwrap());
    assert!(repository
        .complete(key, attempt, "持久化译文")
        .await
        .unwrap());
    assert_eq!(
        second_instance.find_ready(key).await.unwrap().as_deref(),
        Some("持久化译文")
    );
    assert!(matches!(
        second_instance.begin(key).await.unwrap(),
        BeginChapterTranslation::Ready(content) if content == "持久化译文"
    ));

    let retry_hash = vec![8_u8; 32];
    let retry_key = ChapterTranslationKey {
        chapter_id,
        source_sha256: &retry_hash,
        profile: "zh-cn-v1",
    };
    let retry_attempt = match repository.begin(retry_key).await.unwrap() {
        BeginChapterTranslation::Acquired { attempt } => attempt,
        outcome => panic!("unexpected first retry-key outcome: {outcome:?}"),
    };
    assert!(repository
        .fail(retry_key, retry_attempt, "provider")
        .await
        .unwrap());
    assert!(matches!(
        repository.begin(retry_key).await.unwrap(),
        BeginChapterTranslation::InProgress { .. }
    ));
    sqlx::query(
        "UPDATE chapter_translations SET retry_after_at = NOW() - INTERVAL '1 second' \
         WHERE chapter_id = $1 AND source_sha256 = $2 AND profile = $3",
    )
    .bind(chapter_id)
    .bind(&retry_hash)
    .bind("zh-cn-v1")
    .execute(&pool)
    .await
    .unwrap();
    let reclaimed_attempt = match second_instance.begin(retry_key).await.unwrap() {
        BeginChapterTranslation::Acquired { attempt } => attempt,
        outcome => panic!("unexpected reclaimed outcome: {outcome:?}"),
    };
    assert_eq!(reclaimed_attempt, retry_attempt + 1);
    assert!(!repository
        .complete(retry_key, retry_attempt, "stale result")
        .await
        .unwrap());
    assert!(second_instance
        .complete(retry_key, reclaimed_attempt, "fresh result")
        .await
        .unwrap());

    let orphan_hash = vec![9_u8; 32];
    let orphan_key = ChapterTranslationKey {
        chapter_id,
        source_sha256: &orphan_hash,
        profile: "zh-cn-v1",
    };
    let orphan_attempt = match repository.begin(orphan_key).await.unwrap() {
        BeginChapterTranslation::Acquired { attempt } => attempt,
        outcome => panic!("unexpected orphan-key outcome: {outcome:?}"),
    };
    assert!(matches!(
        second_instance.begin(orphan_key).await.unwrap(),
        BeginChapterTranslation::InProgress { .. }
    ));
    sqlx::query(
        "UPDATE chapter_translations SET lease_expires_at = NOW() - INTERVAL '1 second' \
         WHERE chapter_id = $1 AND source_sha256 = $2 AND profile = $3",
    )
    .bind(chapter_id)
    .bind(&orphan_hash)
    .bind("zh-cn-v1")
    .execute(&pool)
    .await
    .unwrap();
    let orphan_reclaimed_attempt = match second_instance.begin(orphan_key).await.unwrap() {
        BeginChapterTranslation::Acquired { attempt } => attempt,
        outcome => panic!("unexpected orphan reclaim outcome: {outcome:?}"),
    };
    assert_eq!(orphan_reclaimed_attempt, orphan_attempt + 1);
    assert!(!repository
        .complete(orphan_key, orphan_attempt, "stale orphan result")
        .await
        .unwrap());
    assert!(second_instance
        .complete(
            orphan_key,
            orphan_reclaimed_attempt,
            "reclaimed orphan result",
        )
        .await
        .unwrap());

    sqlx::query("DELETE FROM chapters WHERE id = $1")
        .bind(chapter_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(repository.find_ready(key).await.unwrap().is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM chapter_translations WHERE chapter_id = $1",
        )
        .bind(chapter_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn narrative_node_first_writer_is_immutable_under_concurrent_and_repeated_save() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "private-node-first-writer").await;
    let novel_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status) \
         VALUES ($1, $2, 'Private branch race', 1, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .execute(&pool)
    .await
    .unwrap();

    let mut first = NarrativeNode::new(
        novel_id,
        1,
        "first candidate".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "first option".into(),
            hint: "first hint".into(),
            generated_consequence: None,
        }],
    )
    .with_anchor_quote("first anchor".into())
    .for_user(user_id);
    first.created_at = chrono::DateTime::from_timestamp_micros(1_700_000_000_000_000).unwrap();
    let mut second = NarrativeNode::new(
        novel_id,
        1,
        "second candidate".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "second option".into(),
            hint: "second hint".into(),
            generated_consequence: None,
        }],
    )
    .with_anchor_quote("second anchor".into())
    .for_user(user_id);
    second.created_at = chrono::DateTime::from_timestamp_micros(1_700_000_001_000_000).unwrap();
    let left_repo = PgNarrativeNodeRepository::new(pool.clone());
    let right_repo = PgNarrativeNodeRepository::new(pool.clone());
    let (left, right) = tokio::join!(
        async {
            left_repo.save(&first).await.unwrap();
            left_repo
                .find_by_chapter(novel_id, 1, Some(user_id))
                .await
                .unwrap()
                .unwrap()
        },
        async {
            right_repo.save(&second).await.unwrap();
            right_repo
                .find_by_chapter(novel_id, 1, Some(user_id))
                .await
                .unwrap()
                .unwrap()
        }
    );

    assert_eq!(left.id, right.id);
    assert_eq!(left.description, right.description);
    assert_eq!(left.anchor_quote, right.anchor_quote);
    assert_eq!(left.choices[0].text, right.choices[0].text);
    assert_eq!(left.created_at, right.created_at);
    let winning_candidate = if left.id == first.id { &first } else { &second };
    assert_eq!(left.id, winning_candidate.id);
    assert_eq!(left.description, winning_candidate.description);
    assert_eq!(left.anchor_quote, winning_candidate.anchor_quote);
    assert_eq!(left.choices[0].text, winning_candidate.choices[0].text);
    assert_eq!(left.created_at, winning_candidate.created_at);

    let losing_candidate = if left.id == first.id { &second } else { &first };
    let repeated_repo = PgNarrativeNodeRepository::new(pool.clone());
    let (first_repeat, second_repeat) = tokio::join!(
        repeated_repo.save(losing_candidate),
        repeated_repo.save(losing_candidate)
    );
    first_repeat.unwrap();
    second_repeat.unwrap();
    let durable = repeated_repo
        .find_by_chapter(novel_id, 1, Some(user_id))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(durable.id, left.id);
    assert_eq!(durable.description, left.description);
    assert_eq!(durable.anchor_quote, left.anchor_quote);
    assert_eq!(durable.choices[0].text, left.choices[0].text);
    assert_eq!(durable.created_at, left.created_at);

    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

async fn poll_import_state(
    pool: &PgPool,
    novel_id: Uuid,
    predicate: impl Fn(&(String, String, String, Option<String>, i64)) -> bool,
) -> (String, String, String, Option<String>, i64) {
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let state = sqlx::query_as::<_, (String, String, String, Option<String>, i64)>(
                "SELECT novel.status::text, job.stage, job.status, job.failure_code, \
                        (SELECT COUNT(*) FROM chapters WHERE novel_id = novel.id) \
                 FROM novels AS novel \
                 JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
                 WHERE novel.id = $1",
            )
            .bind(novel_id)
            .fetch_one(pool)
            .await
            .unwrap();
            if predicate(&state) {
                break state;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("import state did not reach the expected condition")
}

#[tokio::test]
async fn retained_import_accepts_at_source_stage_and_replays_chapters() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "retained-replay").await;
    let text = retained_novel_text();
    let (storage, gate) = FakeSourceStorage::gated(bytes::Bytes::from(text.clone()));
    let handler = blocking_import_handler(&pool, Some(storage));
    let novel_id = handler
        .handle_import(ImportNovelCommand {
            user_id,
            title: "Retained replay".into(),
            author: None,
            raw_content: Some(text),
            source_bytes: Some(bytes::Bytes::from(retained_novel_text())),
            deviation_mode: None,
        })
        .await
        .unwrap();

    // Acceptance committed the source-stage boundary; the worker is parked on
    // the gated read, so no chapters exist yet and the stage is still `source`.
    let state = poll_import_state(&pool, novel_id, |state| state.1 == "source").await;
    assert_eq!(
        state,
        (
            "parsing".into(),
            "source".into(),
            "in_progress".into(),
            None,
            0
        )
    );

    gate.add_permits(1);
    let state = poll_import_state(&pool, novel_id, |state| state.1 == "chapters").await;
    assert_eq!(
        state,
        (
            "parsing".into(),
            "chapters".into(),
            "in_progress".into(),
            None,
            2
        )
    );
    let chapters: Vec<(i32, String)> = sqlx::query_as(
        "SELECT chapter_number, content FROM chapters \
         WHERE novel_id = $1 ORDER BY chapter_number",
    )
    .bind(novel_id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(chapters.len(), 2);
    assert_eq!(chapters[0].0, 1);
    assert!(chapters[0].1.contains("山门之前"));
    assert_eq!(chapters[1].0, 2);
    assert!(chapters[1].1.contains("北方冰原"));
}

#[tokio::test]
async fn source_stage_jobs_are_claimable_at_the_source_boundary() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "source-claim").await;
    let mut novel = Novel::create(user_id, "Source claim".into(), None);
    novel.retain_source_file(format!("source-files/{user_id}/{}", novel.id));
    let repo = NovelPgRepository::new(pool.clone());
    repo.create_source_import(&novel).await.unwrap();

    let candidates = repo.recoverable_imports(100).await.unwrap();
    assert!(
        candidates
            .iter()
            .any(|candidate| candidate.novel_id == novel.id),
        "a pending source-stage job must be a recovery candidate"
    );
    // The claim is fenced by (novel_id, attempt) and treats `source` like any
    // other resumable stage; the claimed worker then replays the object.
    let claim = repo.claim_import(novel.id, user_id).await.unwrap().unwrap();
    assert_eq!(claim.stage, ImportStage::Source);
    assert!(claim.attempt >= 1);
}

/// Seed a source-stage import that already failed once, so the retry endpoint
/// (not a recovery scan) is the only actor that can resume it. This keeps the
/// test deterministic while other tests' recovery loops run concurrently.
async fn seed_failed_source_import(pool: &PgPool, user_id: Uuid, novel: &Novel) {
    let repo = NovelPgRepository::new(pool.clone());
    repo.create_source_import(novel).await.unwrap();
    let claim = repo.claim_import(novel.id, user_id).await.unwrap().unwrap();
    assert_eq!(claim.stage, ImportStage::Source);
    assert!(repo
        .fail_import(novel.id, claim.attempt, "seeded_failure", "seeded")
        .await
        .unwrap());
}

#[tokio::test]
async fn source_stage_missing_object_fails_with_reupload_guidance() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "source-missing").await;
    let mut novel = Novel::create(user_id, "Missing source".into(), None);
    novel.retain_source_file(format!("source-files/{user_id}/{}", novel.id));
    seed_failed_source_import(&pool, user_id, &novel).await;

    let storage = Arc::new(FakeSourceStorage::with_bytes(None));
    let handler = blocking_import_handler(&pool, Some(storage));
    handler.retry_import(user_id, novel.id).await.unwrap();
    let state = poll_import_state(&pool, novel.id, |state| state.2 == "failed").await;
    assert_eq!(
        state,
        (
            "error".into(),
            "source".into(),
            "failed".into(),
            Some("source_missing".into()),
            0,
        )
    );
    let parse_error: Option<String> =
        sqlx::query_scalar("SELECT parse_error FROM novels WHERE id = $1")
            .bind(novel.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        parse_error.as_deref(),
        Some("The retained source file is missing; re-upload the source")
    );
}

#[tokio::test]
async fn source_stage_storage_failure_marks_a_retryable_error() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "source-storage-down").await;
    let mut novel = Novel::create(user_id, "Storage down".into(), None);
    novel.retain_source_file(format!("source-files/{user_id}/{}", novel.id));
    seed_failed_source_import(&pool, user_id, &novel).await;

    let storage = Arc::new(FakeSourceStorage::failing());
    let handler = blocking_import_handler(&pool, Some(storage));
    handler.retry_import(user_id, novel.id).await.unwrap();
    let state = poll_import_state(&pool, novel.id, |state| state.2 == "failed").await;
    assert_eq!(
        state,
        (
            "error".into(),
            "source".into(),
            "failed".into(),
            Some("source_storage_unavailable".into()),
            0,
        )
    );
    let parse_error: Option<String> =
        sqlx::query_scalar("SELECT parse_error FROM novels WHERE id = $1")
            .bind(novel.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        parse_error.as_deref(),
        Some("Source storage is unavailable; retry the import")
    );
}

#[tokio::test]
async fn replayed_chapter_replacement_is_fenced_and_replaces_legacy_rows() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "replay-fence").await;
    let mut novel = Novel::create(user_id, "Fenced replay".into(), None);
    novel.retain_source_file(format!("source-files/{user_id}/{}", novel.id));
    let repo = NovelPgRepository::new(pool.clone());
    repo.create_source_import(&novel).await.unwrap();
    let claim = repo.claim_import(novel.id, user_id).await.unwrap().unwrap();
    assert_eq!(claim.stage, ImportStage::Source);

    // Legacy partial/gapped chapters that the replay must replace.
    sqlx::query(
        "INSERT INTO chapters (id, novel_id, chapter_number, content) \
         VALUES ($1, $2, 1, 'Legacy first chapter'), ($3, $2, 3, 'Legacy gapped chapter')",
    )
    .bind(Uuid::new_v4())
    .bind(novel.id)
    .bind(Uuid::new_v4())
    .execute(&pool)
    .await
    .unwrap();

    let replayed = vec![
        Chapter::new(novel.id, 1, None, "Replayed first chapter".repeat(4)),
        Chapter::new(novel.id, 2, None, "Replayed second chapter".repeat(4)),
    ];
    // A stale attempt cannot replace chapters or advance the stage.
    assert!(!repo
        .replace_import_chapters(novel.id, claim.attempt + 1, &replayed)
        .await
        .unwrap());
    let stage: String =
        sqlx::query_scalar("SELECT stage FROM novel_import_jobs WHERE novel_id = $1")
            .bind(novel.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stage, "source");
    let legacy_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chapters WHERE novel_id = $1")
        .bind(novel.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(legacy_count, 2);

    assert!(repo
        .replace_import_chapters(novel.id, claim.attempt, &replayed)
        .await
        .unwrap());
    let state: (String, i64) = sqlx::query_as(
        "SELECT job.stage, (SELECT COUNT(*) FROM chapters WHERE novel_id = novel.id) \
         FROM novels AS novel JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
         WHERE novel.id = $1",
    )
    .bind(novel.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, ("chapters".into(), 2));
    let contents: Vec<String> = sqlx::query_scalar(
        "SELECT content FROM chapters WHERE novel_id = $1 ORDER BY chapter_number",
    )
    .bind(novel.id)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert!(contents[0].starts_with("Replayed first chapter"));
    assert!(contents[1].starts_with("Replayed second chapter"));

    let repaired = vec![
        Chapter::new(novel.id, 1, None, "Repaired first chapter".repeat(4)),
        Chapter::new(novel.id, 2, None, "Repaired second chapter".repeat(4)),
        Chapter::new(novel.id, 3, None, "Repaired third chapter".repeat(4)),
    ];
    assert!(repo
        .replace_import_chapters(novel.id, claim.attempt, &repaired)
        .await
        .unwrap());
    let repaired_state: (String, i64) = sqlx::query_as(
        "SELECT job.stage, (SELECT COUNT(*) FROM chapters WHERE novel_id = novel.id) \
         FROM novels AS novel JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
         WHERE novel.id = $1",
    )
    .bind(novel.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(repaired_state, ("chapters".into(), 3));
}

#[tokio::test]
async fn failed_source_import_retries_from_the_retained_object() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "source-retry").await;
    let mut novel = Novel::create(user_id, "Retried source".into(), None);
    novel.retain_source_file(format!("source-files/{user_id}/{}", novel.id));
    seed_failed_source_import(&pool, user_id, &novel).await;

    // The retained object becomes readable; the retry endpoint resumes the
    // import without a new upload.
    let storage = Arc::new(FakeSourceStorage::with_bytes(Some(bytes::Bytes::from(
        retained_novel_text(),
    ))));
    let handler = blocking_import_handler(&pool, Some(storage));
    handler.retry_import(user_id, novel.id).await.unwrap();
    let state = poll_import_state(&pool, novel.id, |state| state.1 == "chapters").await;
    assert_eq!(
        state,
        (
            "parsing".into(),
            "chapters".into(),
            "in_progress".into(),
            None,
            2
        )
    );
}

#[tokio::test]
async fn import_claims_are_capped_and_terminate_with_budget_exhausted() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = insert_test_user(&pool, "import-budget").await;
    let novel = Novel::create(user_id, "Budget ceiling".into(), None);
    let chapter = Chapter::new(
        novel.id,
        1,
        None,
        "A durable source chapter for the budget ceiling.".repeat(4),
    );
    let repo = NovelPgRepository::new(pool.clone());
    repo.create_import(&novel, &[chapter]).await.unwrap();

    for expected_attempt in 1..=MAX_IMPORT_ATTEMPTS {
        let claim = repo.claim_import(novel.id, user_id).await.unwrap().unwrap();
        assert_eq!(claim.attempt, expected_attempt);
        assert!(repo
            .fail_import(novel.id, expected_attempt, "seeded_failure", "seeded")
            .await
            .unwrap());
    }

    // The (ceiling+1)-th claim terminates the job instead of issuing work.
    assert!(repo
        .claim_import(novel.id, user_id)
        .await
        .unwrap()
        .is_none());
    let state: (String, Option<String>, String, Option<String>) = sqlx::query_as(
        "SELECT job.status, job.failure_code, novel.status::text, novel.parse_error \
         FROM novel_import_jobs AS job JOIN novels AS novel ON novel.id = job.novel_id \
         WHERE novel.id = $1",
    )
    .bind(novel.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        state,
        (
            "failed".into(),
            Some("budget_exhausted".into()),
            "error".into(),
            Some(IMPORT_BUDGET_EXHAUSTED_MESSAGE.into()),
        )
    );

    // Recovery never reclaims a budget-exhausted job, and the terminal state
    // is idempotent.
    let candidates = repo.recoverable_imports(100).await.unwrap();
    assert!(!candidates.iter().any(|c| c.novel_id == novel.id));
    assert!(repo
        .claim_import(novel.id, user_id)
        .await
        .unwrap()
        .is_none());
    let unchanged: (String, Option<String>, String, Option<String>) = sqlx::query_as(
        "SELECT job.status, job.failure_code, novel.status::text, novel.parse_error \
         FROM novel_import_jobs AS job JOIN novels AS novel ON novel.id = job.novel_id \
         WHERE novel.id = $1",
    )
    .bind(novel.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(state, unchanged);

    // The retry endpoint surfaces the guidance without any provider call.
    let handler = blocking_import_handler(&pool, None);
    assert_eq!(
        handler
            .retry_import(user_id, novel.id)
            .await
            .unwrap_err()
            .to_string(),
        IMPORT_BUDGET_EXHAUSTED_MESSAGE
    );
}

#[tokio::test]
async fn complete_import_rejects_gapped_chapters_with_matching_count() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();
    let (_, novel_id) =
        seed_gapped_novel(&pool, "parsing", Some(("enriched", "in_progress"))).await;
    sqlx::query("INSERT INTO characters (novel_id, name) VALUES ($1, 'Gapped hero')")
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO canon_story_models \
             (novel_id, model_version, schema_version, prompt_version, content) \
         VALUES ($1, 1, 1, 'gapped-import-test-v1', '{}'::jsonb)",
    )
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();

    let repo = NovelPgRepository::new(pool.clone());
    assert!(repo.complete_import(novel_id, 1).await.is_err());
    assert_eq!(
        sqlx::query_scalar::<_, String>("SELECT status::text FROM novels WHERE id = $1")
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        "parsing"
    );

    // Contiguous again, but tabs and newlines survive a default BTRIM(): a
    // chapter that carries no readable character must still block publication.
    sqlx::query(
        "UPDATE chapters SET chapter_number = 2, content = $2 \
         WHERE novel_id = $1 AND chapter_number = 3",
    )
    .bind(novel_id)
    .bind("\t\n \t")
    .execute(&pool)
    .await
    .unwrap();
    assert!(repo.complete_import(novel_id, 1).await.is_err());

    // Non-ASCII whitespace survives a POSIX [:space:] test under LC_CTYPE=C,
    // while Rust str::trim() strips it: publication must agree with Rust
    // regardless of the database locale.
    sqlx::query("UPDATE chapters SET content = $2 WHERE novel_id = $1 AND chapter_number = 2")
        .bind(novel_id)
        .bind("\u{00a0}\u{3000}")
        .execute(&pool)
        .await
        .unwrap();
    assert!(repo.complete_import(novel_id, 1).await.is_err());

    sqlx::query(
        "UPDATE chapters SET content = 'A durable second chapter.' \
         WHERE novel_id = $1 AND chapter_number = 2",
    )
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(repo.complete_import(novel_id, 1).await.unwrap());
}

#[tokio::test]
async fn import_character_snapshot_is_atomic_and_attempt_fenced() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("character-snapshot-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();

    let novel = Novel::create(user_id, "Character snapshot".into(), None);
    let chapters = vec![Chapter::new(
        novel.id,
        1,
        Some("Chapter 1".into()),
        "A source-backed chapter long enough for character snapshot testing.".repeat(4),
    )];
    let novel_repo = NovelPgRepository::new(pool.clone());
    novel_repo.create_import(&novel, &chapters).await.unwrap();
    let first = novel_repo
        .claim_import(novel.id, user_id)
        .await
        .unwrap()
        .unwrap();

    let alice = Character::new(novel.id, "Alice".into(), CharacterRole::Protagonist);
    let bob = Character::new(novel.id, "Bob".into(), CharacterRole::Supporting);
    let relationship = CharacterRelationshipRecord {
        id: Uuid::new_v4(),
        novel_id: novel.id,
        from_character_id: alice.id,
        to_character_id: bob.id,
        relationship_type: "ally".into(),
        description: Some("Source-backed allies".into()),
        strength: 80,
    };
    let character_repo = CharacterPgRepository::new(pool.clone());
    assert!(character_repo
        .replace_import(
            novel.id,
            first.attempt,
            &[alice.clone(), bob],
            &[relationship],
        )
        .await
        .unwrap());

    sqlx::query(
        "UPDATE novel_import_jobs \
         SET lease_expires_at = NOW() - INTERVAL '1 second' \
         WHERE novel_id = $1",
    )
    .bind(novel.id)
    .execute(&pool)
    .await
    .unwrap();
    let second = novel_repo
        .claim_import(novel.id, user_id)
        .await
        .unwrap()
        .unwrap();

    let stale = Character::new(novel.id, "Stale".into(), CharacterRole::Supporting);
    assert!(!character_repo
        .replace_import(novel.id, first.attempt, &[stale], &[])
        .await
        .unwrap());
    assert_eq!(
        character_repo
            .find_by_novel(novel.id)
            .await
            .unwrap()
            .iter()
            .map(|character| character.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Alice", "Bob"]
    );
    assert_eq!(
        character_repo
            .find_relationships(novel.id)
            .await
            .unwrap()
            .len(),
        1
    );

    let current = Character::new(novel.id, "Current".into(), CharacterRole::Protagonist);
    assert!(character_repo
        .replace_import(novel.id, second.attempt, &[current], &[])
        .await
        .unwrap());

    let candidate_a = Character::new(novel.id, "Candidate A".into(), CharacterRole::Protagonist);
    let candidate_b = Character::new(novel.id, "Candidate B".into(), CharacterRole::Supporting);
    let duplicate_id = Uuid::new_v4();
    let duplicate_relationships = [
        CharacterRelationshipRecord {
            id: duplicate_id,
            novel_id: novel.id,
            from_character_id: candidate_a.id,
            to_character_id: candidate_b.id,
            relationship_type: "ally".into(),
            description: None,
            strength: 80,
        },
        CharacterRelationshipRecord {
            id: duplicate_id,
            novel_id: novel.id,
            from_character_id: candidate_b.id,
            to_character_id: candidate_a.id,
            relationship_type: "ally".into(),
            description: None,
            strength: 80,
        },
    ];
    assert!(character_repo
        .replace_import(
            novel.id,
            second.attempt,
            &[candidate_a, candidate_b],
            &duplicate_relationships,
        )
        .await
        .is_err());
    assert_eq!(
        character_repo
            .find_by_novel(novel.id)
            .await
            .unwrap()
            .iter()
            .map(|character| character.name.as_str())
            .collect::<Vec<_>>(),
        vec!["Current"]
    );
    assert!(character_repo
        .find_relationships(novel.id)
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn source_file_cleanup_waits_for_shared_novel_deletion() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("source-file-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();

    let mut novel = Novel::create(user_id, "Stored source".into(), None);
    let object_key = format!("source-files/{user_id}/{}", novel.id);
    novel.retain_source_file(object_key.clone());
    let deletions = PgSourceFileDeletionRepository::new(pool.clone());
    deletions
        .enqueue(
            &object_key,
            chrono::Utc::now() + chrono::Duration::minutes(5),
        )
        .await
        .unwrap();
    let chapter = Chapter::new(
        novel.id,
        1,
        None,
        "A retained source chapter long enough for the cleanup contract.".repeat(3),
    );
    NovelPgRepository::new(pool.clone())
        .create_import(&novel, &[chapter])
        .await
        .unwrap();
    let unmanaged_keys = [
        "legacy-source.txt".to_owned(),
        format!("source-files/{}", "x".repeat(1_024)),
    ];
    for unmanaged_key in &unmanaged_keys {
        sqlx::query(
            "INSERT INTO novels (id, user_id, title, original_file_key) VALUES ($1, $2, $3, $4)",
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind("Unmanaged source")
        .bind(unmanaged_key)
        .execute(&pool)
        .await
        .unwrap();
    }
    assert!(!sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM source_file_deletions WHERE object_key = $1)",
    )
    .bind(&object_key)
    .fetch_one(&pool)
    .await
    .unwrap());

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(!deletions
        .due(100)
        .await
        .unwrap()
        .iter()
        .any(|pending| pending.object_key == object_key));
    assert!(
        sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM novels WHERE id = $1)",)
            .bind(novel.id)
            .fetch_one(&pool)
            .await
            .unwrap()
    );
    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(novel.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(deletions
        .due(100)
        .await
        .unwrap()
        .iter()
        .any(|pending| pending.object_key == object_key));
    for unmanaged_key in unmanaged_keys {
        assert!(!sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM source_file_deletions WHERE object_key = $1)",
        )
        .bind(unmanaged_key)
        .fetch_one(&pool)
        .await
        .unwrap());
    }
    deletions.complete(&object_key).await.unwrap();
}

#[tokio::test]
async fn canon_story_models_are_versioned_and_immutable() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("canon-contract-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status) \
         VALUES ($1, $2, $3, 1, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind("Canon contract")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO chapters (novel_id, chapter_number, content) VALUES ($1, 1, $2)")
        .bind(novel_id)
        .bind("The journey begins.")
        .execute(&pool)
        .await
        .unwrap();
    let character_id = Uuid::new_v4();
    sqlx::query("INSERT INTO characters (id, novel_id, name) VALUES ($1, $2, 'Hero')")
        .bind(character_id)
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();

    let source = SourceEvidence {
        provenance: vec![SourceCitation {
            chapter_number: 1,
            excerpt: "The journey begins.".into(),
        }],
        confidence: 1.0,
    };
    let mut model = CanonStoryModel {
        id: Uuid::new_v4(),
        novel_id,
        model_version: 1,
        schema_version: CANON_STORY_SCHEMA_VERSION,
        prompt_version: "canon-extraction-v1".into(),
        content: CanonStoryContent {
            arcs: vec![StoryArc {
                id: "arc-1".into(),
                title: "Journey".into(),
                summary: "The mainline.".into(),
                event_ids: vec!["event-1".into()],
                evidence: source.clone(),
            }],
            events: vec![CanonEvent {
                id: "event-1".into(),
                sequence: 1,
                summary: "The journey begins.".into(),
                caused_by: vec![],
                location_ids: vec![],
                character_ids: vec![character_id],
                faction_ids: vec![],
                evidence: source.clone(),
            }],
            locations: vec![],
            factions: vec![],
            world_rules: vec![],
            character_goals: vec![],
            relationships: vec![],
            deaths: vec![],
            unresolved_threads: vec![],
            ending: CanonEndingSnapshot {
                summary: "The journey ends.".into(),
                character_states: std::collections::BTreeMap::from([(
                    character_id,
                    "The hero reaches the ending.".into(),
                )]),
                faction_states: Default::default(),
                location_states: Default::default(),
                unresolved_thread_ids: vec![],
                evidence: source,
            },
        },
        created_at: std::time::SystemTime::UNIX_EPOCH.into(),
    };
    let repository = PgCanonStoryModelRepository::new(pool.clone());
    sqlx::query("UPDATE novels SET total_chapters = 2, status = 'parsing' WHERE id = $1")
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novel_import_jobs ( \
             novel_id, stage, status, attempt, lease_expires_at \
         ) VALUES ($1, 'enriched', 'in_progress', 2, NOW() + INTERVAL '2 minutes')",
    )
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(repository.insert_import(&model, 2).await.is_err());
    sqlx::query("UPDATE novels SET total_chapters = 1 WHERE id = $1")
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    let mut fabricated = model.clone();
    fabricated.content.events[0].evidence.provenance[0].excerpt = "Invented evidence".into();
    assert!(repository.insert_import(&fabricated, 2).await.is_err());
    let checkpoint_json = r#"{"coverage_summary":"The journey begins."}"#;
    let checkpoint_source = "x".repeat(16_000);
    assert!(!repository
        .save_import_checkpoint(
            CanonExtractionCheckpoint {
                novel_id,
                model_version: 1,
                prompt_version: "canon-chunk-v3",
                chapter_number: 1,
                chunk_index: 0,
                is_final: true,
                source_content: &checkpoint_source,
                extraction_json: checkpoint_json,
            },
            1,
        )
        .await
        .unwrap());
    assert!(repository
        .save_import_checkpoint(
            CanonExtractionCheckpoint {
                novel_id,
                model_version: 1,
                prompt_version: "canon-chunk-v3",
                chapter_number: 1,
                chunk_index: 0,
                is_final: true,
                source_content: &checkpoint_source,
                extraction_json: checkpoint_json,
            },
            2,
        )
        .await
        .unwrap());
    assert_eq!(
        repository
            .find_import_checkpoint(novel_id, 1, "canon-chunk-v3", 1, 0, &checkpoint_source)
            .await
            .unwrap(),
        Some(checkpoint_json.into())
    );
    assert!(repository
        .find_import_checkpoint(novel_id, 1, "canon-chunk-v3", 1, 0, "changed source")
        .await
        .unwrap()
        .is_none());
    assert!(repository.insert_import(&model, 2).await.unwrap());
    assert!(repository
        .find_import_checkpoint(novel_id, 1, "canon-chunk-v3", 1, 0, &checkpoint_source)
        .await
        .unwrap()
        .is_none());
    assert_eq!(
        repository.find_version(novel_id, 1).await.unwrap(),
        Some(model.clone())
    );

    model.id = Uuid::new_v4();
    model.model_version = 2;
    assert!(repository.insert_import(&model, 2).await.unwrap());
    assert_eq!(
        repository.find_latest(novel_id).await.unwrap(),
        Some(model.clone())
    );
    model.id = Uuid::new_v4();
    assert!(repository.insert_import(&model, 2).await.is_err());

    model.id = Uuid::new_v4();
    model.model_version = 3;
    assert!(!repository.insert_import(&model, 1).await.unwrap());
    assert!(repository
        .find_version(novel_id, 3)
        .await
        .unwrap()
        .is_none());
    assert!(repository.insert_import(&model, 2).await.unwrap());

    let immutable_error =
        sqlx::query("UPDATE canon_story_models SET prompt_version = 'changed' WHERE novel_id = $1")
            .bind(novel_id)
            .execute(&pool)
            .await
            .unwrap_err();
    assert_eq!(
        immutable_error
            .as_database_error()
            .and_then(|error| error.code()),
        Some(std::borrow::Cow::Borrowed("55000"))
    );
}

fn test_game_rule_template(novel_id: Uuid) -> GameRuleTemplate {
    let attributes = ["vigor", "insight", "influence"]
        .into_iter()
        .map(|key| GameAttribute {
            key: key.into(),
            label: key.into(),
            description: format!("{key} in this novel"),
            default_score: 10,
            source_chapters: vec![1],
        })
        .collect::<Vec<_>>();
    let action_rules = GameActionKind::ALL
        .into_iter()
        .enumerate()
        .map(|(index, kind)| GameActionRule {
            kind,
            attribute_key: attributes[index % attributes.len()].key.clone(),
            difficulty_class: 12,
            description: "Resolve an uncertain action".into(),
            source_chapters: vec![1],
        })
        .collect();
    GameRuleTemplate::new(novel_id, 1, attributes, action_rules).unwrap()
}

async fn seed_game_rule_model(pool: &PgPool, label: &str) -> (Uuid, Uuid) {
    let user_id = insert_test_user(pool, label).await;
    let novel_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status) \
         VALUES ($1, $2, $3, 1, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind(format!("{label} novel"))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO canon_story_models \
         (novel_id, model_version, schema_version, prompt_version, content) \
         VALUES ($1, 1, 1, 'game-rule-contract-v1', '{}'::jsonb)",
    )
    .bind(novel_id)
    .execute(pool)
    .await
    .unwrap();
    (user_id, novel_id)
}

#[tokio::test]
async fn game_rule_generation_is_single_owner_fenced_bounded_and_immutable() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id) = seed_game_rule_model(&pool, "game-rule-contract").await;
    let repository = PgCanonStoryModelRepository::new(pool.clone());

    let (first, second) = tokio::join!(
        repository.begin_game_rule_generation(novel_id, 1),
        repository.begin_game_rule_generation(novel_id, 1),
    );
    let outcomes = [first.unwrap(), second.unwrap()];
    let attempt = outcomes
        .iter()
        .find_map(|outcome| match outcome {
            BeginGameRuleGeneration::Acquired { attempt } => Some(*attempt),
            _ => None,
        })
        .expect("one concurrent caller must acquire the generation lease");
    assert_eq!(attempt, 1);
    assert_eq!(
        outcomes
            .iter()
            .filter(|outcome| matches!(outcome, BeginGameRuleGeneration::InProgress { .. }))
            .count(),
        1,
    );
    assert!(repository
        .renew_game_rule_generation(novel_id, 1, attempt)
        .await
        .unwrap());

    sqlx::query(
        "UPDATE novel_game_rule_templates \
         SET lease_expires_at = NOW() - INTERVAL '1 second' \
         WHERE novel_id = $1 AND canon_model_version = 1",
    )
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let reclaimed = match repository
        .begin_game_rule_generation(novel_id, 1)
        .await
        .unwrap()
    {
        BeginGameRuleGeneration::Acquired { attempt } => attempt,
        outcome => panic!("unexpected reclaim outcome: {outcome:?}"),
    };
    assert_eq!(reclaimed, 2);
    assert!(!repository
        .complete_game_rule_generation(&test_game_rule_template(novel_id), attempt)
        .await
        .unwrap());
    assert!(repository
        .fail_game_rule_generation(novel_id, 1, reclaimed, "provider_failed")
        .await
        .unwrap());
    let final_attempt = match repository
        .begin_game_rule_generation(novel_id, 1)
        .await
        .unwrap()
    {
        BeginGameRuleGeneration::Acquired { attempt } => attempt,
        outcome => panic!("unexpected final claim outcome: {outcome:?}"),
    };
    assert_eq!(final_attempt, 3);
    let template = test_game_rule_template(novel_id);
    assert!(repository
        .complete_game_rule_generation(&template, final_attempt)
        .await
        .unwrap());
    assert_eq!(
        repository
            .find_game_rule_template(novel_id, 1)
            .await
            .unwrap(),
        Some(template.clone()),
    );
    assert!(matches!(
        repository
            .begin_game_rule_generation(novel_id, 1)
            .await
            .unwrap(),
        BeginGameRuleGeneration::Ready(found) if found == template
    ));
    let immutable_error = sqlx::query(
        "UPDATE novel_game_rule_templates SET prompt_version = 'changed' \
         WHERE novel_id = $1 AND canon_model_version = 1",
    )
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap_err();
    assert_eq!(
        immutable_error
            .as_database_error()
            .and_then(|error| error.code()),
        Some(std::borrow::Cow::Borrowed("55000")),
    );

    let (exhausted_user, exhausted_novel) =
        seed_game_rule_model(&pool, "game-rule-exhaustion").await;
    for expected_attempt in 1..=2 {
        let claimed = repository
            .begin_game_rule_generation(exhausted_novel, 1)
            .await
            .unwrap();
        let attempt = match claimed {
            BeginGameRuleGeneration::Acquired { attempt } => attempt,
            outcome => panic!("unexpected budget claim: {outcome:?}"),
        };
        assert_eq!(attempt, expected_attempt);
        assert!(repository
            .fail_game_rule_generation(exhausted_novel, 1, attempt, "invalid_output")
            .await
            .unwrap());
    }
    let final_claim = repository
        .begin_game_rule_generation(exhausted_novel, 1)
        .await
        .unwrap();
    assert!(matches!(
        final_claim,
        BeginGameRuleGeneration::Acquired { attempt: 3 }
    ));
    sqlx::query(
        "UPDATE novel_game_rule_templates \
         SET lease_expires_at = NOW() - INTERVAL '1 second' \
         WHERE novel_id = $1 AND canon_model_version = 1",
    )
    .bind(exhausted_novel)
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        repository
            .begin_game_rule_generation(exhausted_novel, 1)
            .await
            .unwrap(),
        BeginGameRuleGeneration::Exhausted
    ));
    assert!(matches!(
        repository
            .begin_game_rule_generation(exhausted_novel, 1)
            .await
            .unwrap(),
        BeginGameRuleGeneration::Exhausted
    ));
    assert_eq!(
        sqlx::query_as::<_, (String, i64, Option<String>)>(
            "SELECT status, attempt, failure_code FROM novel_game_rule_templates \
             WHERE novel_id = $1 AND canon_model_version = 1",
        )
        .bind(exhausted_novel)
        .fetch_one(&pool)
        .await
        .unwrap(),
        ("failed".into(), 3, Some("budget_exhausted".into())),
    );

    for cleanup_user in [user_id, exhausted_user] {
        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(cleanup_user)
            .execute(&pool)
            .await
            .unwrap();
    }
}

async fn wait_for_blocked_query(pool: &PgPool, needle: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let blocked: bool = sqlx::query_scalar(
                "SELECT EXISTS (\
                    SELECT 1 FROM pg_stat_activity \
                    WHERE datname = current_database() \
                      AND wait_event_type = 'Lock' \
                      AND POSITION($1 IN query) > 0\
                )",
            )
            .bind(needle)
            .fetch_one(pool)
            .await
            .unwrap();
            if blocked {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("repository query did not reach the expected row lock");
}

#[tokio::test]
async fn reading_progress_creation_and_identity_transition_are_atomic() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let future_character_id = Uuid::new_v4();
    let visible_character_id = Uuid::new_v4();
    let unknown_character_id = Uuid::new_v4();
    let invalid_character_id = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("progress-contract-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, deviation_mode) VALUES ($1, $2, $3, 5, 'creative')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind("Progress contract")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO characters (id, novel_id, name, first_appearance_chapter) VALUES ($1, $2, 'Future', 5), ($3, $2, 'Unknown', NULL), ($4, $2, 'Invalid', 0), ($5, $2, 'Visible', 1)",
    )
    .bind(future_character_id)
    .bind(novel_id)
    .bind(unknown_character_id)
    .bind(invalid_character_id)
    .bind(visible_character_id)
    .execute(&pool)
    .await
    .unwrap();

    let repository = PgReadingProgressRepository::new(pool.clone());
    let (left, right) = tokio::join!(
        repository.get_or_create(user_id, novel_id, "creative"),
        repository.get_or_create(user_id, novel_id, "creative")
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.id, right.id);
    assert_eq!(left.deviation_mode, "creative");
    assert_eq!(
        repository
            .get_or_create(user_id, novel_id, "remix")
            .await
            .unwrap()
            .deviation_mode,
        "creative"
    );
    let count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM reading_progress WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count.0, 1);

    repository
        .update_chapter(user_id, novel_id, 5)
        .await
        .unwrap();
    repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Future"),
            Some(future_character_id),
        )
        .await
        .unwrap();
    repository
        .update_chapter(user_id, novel_id, 1)
        .await
        .unwrap();
    let rewound = repository
        .get_or_create(user_id, novel_id, "creative")
        .await
        .unwrap();
    assert_eq!(rewound.current_chapter, 1);
    assert_eq!(rewound.reader_identity_type, "self");
    assert!(rewound.reader_identity.is_none());
    assert!(rewound.reader_character_id.is_none());
    assert!(repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Unknown"),
            Some(unknown_character_id),
        )
        .await
        .is_err());

    repository
        .update_chapter(user_id, novel_id, 5)
        .await
        .unwrap();
    repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Visible"),
            Some(visible_character_id),
        )
        .await
        .unwrap();

    let mut identity_first = pool.begin().await.unwrap();
    sqlx::query(
        "UPDATE reading_progress \
         SET reader_identity_type = 'character', reader_identity = 'Future', reader_character_id = $3 \
         WHERE user_id = $1 AND novel_id = $2 AND current_chapter = 5",
    )
    .bind(user_id)
    .bind(novel_id)
    .bind(future_character_id)
    .execute(&mut *identity_first)
    .await
    .unwrap();
    let rewind_repository = PgReadingProgressRepository::new(pool.clone());
    let rewind =
        tokio::spawn(async move { rewind_repository.update_chapter(user_id, novel_id, 1).await });
    wait_for_blocked_query(&pool, "locked_progress AS MATERIALIZED").await;
    identity_first.commit().await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), rewind)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let identity_first_result = repository
        .get_or_create(user_id, novel_id, "creative")
        .await
        .unwrap();
    assert_eq!(identity_first_result.current_chapter, 1);
    assert_eq!(identity_first_result.reader_identity_type, "self");
    assert!(identity_first_result.reader_character_id.is_none());

    repository
        .update_chapter(user_id, novel_id, 5)
        .await
        .unwrap();
    repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Visible"),
            Some(visible_character_id),
        )
        .await
        .unwrap();
    let mut rewind_first = pool.begin().await.unwrap();
    sqlx::query(
        "SELECT id FROM reading_progress \
         WHERE user_id = $1 AND novel_id = $2 FOR UPDATE",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&mut *rewind_first)
    .await
    .unwrap();
    sqlx::query(
        "UPDATE reading_progress SET current_chapter = 1 \
         WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&mut *rewind_first)
    .await
    .unwrap();
    let identity_repository = PgReadingProgressRepository::new(pool.clone());
    let set_future = tokio::spawn(async move {
        identity_repository
            .set_identity(
                user_id,
                novel_id,
                "character",
                Some("Future"),
                Some(future_character_id),
            )
            .await
    });
    wait_for_blocked_query(&pool, "SET reader_identity_type = $3::identity_type").await;
    rewind_first.commit().await.unwrap();
    let set_result = tokio::time::timeout(std::time::Duration::from_secs(2), set_future)
        .await
        .unwrap()
        .unwrap();
    assert!(set_result.is_err());
    let rewind_first_result = repository
        .get_or_create(user_id, novel_id, "creative")
        .await
        .unwrap();
    assert_eq!(rewind_first_result.current_chapter, 1);
    assert_eq!(
        rewind_first_result.reader_identity.as_deref(),
        Some("Visible")
    );
    assert_eq!(
        rewind_first_result.reader_character_id,
        Some(visible_character_id)
    );
    assert!(repository
        .set_identity(
            user_id,
            novel_id,
            "character",
            Some("Invalid"),
            Some(invalid_character_id),
        )
        .await
        .is_err());

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn permanent_candidate_buckets_bound_legacy_without_starving_uuid_v5() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let character_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("memory-buckets-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO novels (id, user_id, title) VALUES ($1, $2, $3)")
        .bind(novel_id)
        .bind(user_id)
        .bind("Memory bucket contract")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO characters (id, novel_id, name) VALUES ($1, $2, $3)")
        .bind(character_id)
        .bind(novel_id)
        .bind("Memory Witness")
        .execute(&pool)
        .await
        .unwrap();

    let repository = PgMemoryRepository::new(pool.clone());
    for index in 0..11 {
        let legacy = Memory::new_permanent(
            character_id,
            user_id,
            novel_id,
            format!("high-importance legacy memory {index}"),
            10,
            1,
        );
        repository.save(&legacy).await.unwrap();
    }
    let source_turn_id = Uuid::new_v4();
    let mut journey_candidate = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "bounded UUIDv5 journey candidate".into(),
        7,
        1,
    );
    journey_candidate.id = Uuid::new_v5(&Uuid::NAMESPACE_OID, source_turn_id.as_bytes());
    repository.save(&journey_candidate).await.unwrap();
    let quarantined_precontract = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "private pre-contract world prose".into(),
        7,
        1,
    );
    repository.save(&quarantined_precontract).await.unwrap();

    let candidates = repository
        .find_permanent_candidates(character_id, user_id, novel_id, 1, 10, 20)
        .await
        .unwrap();
    assert_eq!(candidates.len(), 12);
    assert!(candidates
        .iter()
        .any(|memory| memory.id == journey_candidate.id));
    assert!(candidates
        .iter()
        .all(|memory| memory.id != quarantined_precontract.id));
    assert_eq!(
        candidates
            .iter()
            .filter(|memory| memory.id.get_version_num() != 5)
            .count(),
        11
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn chat_history_is_scoped_by_the_committed_reader_identity() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let conversation_character_id = Uuid::new_v4();
    let reader_a = Uuid::new_v4();
    let reader_b = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("identity-history-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO novels (id, user_id, title) VALUES ($1, $2, $3)")
        .bind(novel_id)
        .bind(user_id)
        .bind("Identity-scoped chat history")
        .execute(&pool)
        .await
        .unwrap();
    for (id, name) in [
        (conversation_character_id, "Conversation Character"),
        (reader_a, "Reader A"),
        (reader_b, "Reader B"),
    ] {
        sqlx::query("INSERT INTO characters (id, novel_id, name) VALUES ($1, $2, $3)")
            .bind(id)
            .bind(novel_id)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }

    let chat_repo = PgChatRepository::new(pool.clone());
    let shared_created_at = chrono::Utc::now();
    for (index, reader_character_id, chapter_context, marker) in [
        (1_u8, None, 1, "SELF-HISTORY-MARKER"),
        (2, Some(reader_a), 1, "CHARACTER-A-FIRST-MARKER"),
        (3, Some(reader_b), 1, "CHARACTER-B-MARKER"),
        (4, Some(reader_a), 1, "CHARACTER-A-CONTINUITY-MARKER"),
        (5, None, 2, "SELF-FUTURE-CHAPTER-MARKER"),
    ] {
        let claim = ChatTurnClaim {
            id: Uuid::new_v4(),
            user_id,
            character_id: conversation_character_id,
            novel_id,
            request_fingerprint: vec![index; 32],
            chapter_context,
            reader_identity: Some(match reader_character_id {
                Some(id) if id == reader_a => "Reader A".into(),
                Some(_) => "Reader B".into(),
                None => "Self Reader".into(),
            }),
            reader_identity_type: if reader_character_id.is_some() {
                "character".into()
            } else {
                "self".into()
            },
            reader_character_id,
            deviation_mode: "canon".into(),
        };
        let attempt = match chat_repo.begin_turn(&claim).await.unwrap() {
            BeginChatTurn::Acquired { attempt, .. } => attempt,
            result => panic!("unexpected turn reservation: {result:?}"),
        };
        let mut user_message = ChatMessage::new(
            user_id,
            conversation_character_id,
            novel_id,
            "user".into(),
            marker.into(),
            claim.reader_identity.clone(),
            Some(chapter_context),
        )
        .with_turn_id(claim.id);
        user_message.created_at = shared_created_at;
        let mut character_message = ChatMessage::new(
            user_id,
            conversation_character_id,
            novel_id,
            "character".into(),
            format!("reply to {marker}"),
            claim.reader_identity.clone(),
            Some(chapter_context),
        )
        .with_turn_id(claim.id);
        character_message.created_at = shared_created_at;
        chat_repo
            .complete_turn(&claim, attempt, &user_message, &character_message)
            .await
            .unwrap();
    }
    sqlx::query(
        "INSERT INTO chat_messages (character_id, user_id, novel_id, role, content, chapter_context) VALUES ($1, $2, $3, 'user', $4, 1)",
    )
    .bind(conversation_character_id)
    .bind(user_id)
    .bind(novel_id)
    .bind("LEGACY-NULL-TURN-MARKER")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chat_messages (character_id, user_id, novel_id, role, content, chapter_context) VALUES ($1, $2, $3, 'user', $4, NULL)",
    )
    .bind(conversation_character_id)
    .bind(user_id)
    .bind(novel_id)
    .bind("LEGACY-NULL-CHAPTER-MARKER")
    .execute(&pool)
    .await
    .unwrap();

    let self_history = chat_repo
        .find_recent(conversation_character_id, user_id, novel_id, None, 1, 20)
        .await
        .unwrap();
    let self_prompt = self_history
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(self_prompt.contains("SELF-HISTORY-MARKER"));
    assert!(self_prompt.contains("LEGACY-NULL-TURN-MARKER"));
    assert!(!self_prompt.contains("SELF-FUTURE-CHAPTER-MARKER"));
    assert!(!self_prompt.contains("LEGACY-NULL-CHAPTER-MARKER"));
    assert!(!self_prompt.contains("CHARACTER-A-FIRST-MARKER"));
    assert!(!self_prompt.contains("CHARACTER-B-MARKER"));

    let character_a_history = chat_repo
        .find_recent(
            conversation_character_id,
            user_id,
            novel_id,
            Some(reader_a),
            1,
            20,
        )
        .await
        .unwrap();
    let character_a_prompt = character_a_history
        .iter()
        .map(|message| message.content.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(character_a_prompt.contains("CHARACTER-A-FIRST-MARKER"));
    assert!(character_a_prompt.contains("CHARACTER-A-CONTINUITY-MARKER"));
    assert!(!character_a_prompt.contains("SELF-HISTORY-MARKER"));
    assert!(!character_a_prompt.contains("CHARACTER-B-MARKER"));
    assert!(!character_a_prompt.contains("LEGACY-NULL-TURN-MARKER"));
    assert_eq!(character_a_history.len(), 4);
    assert!(character_a_history.chunks_exact(2).all(|turn| {
        turn[0].turn_id == turn[1].turn_id && turn[0].role == "user" && turn[1].role == "character"
    }));
    let character_a_latest = chat_repo
        .find_recent(
            conversation_character_id,
            user_id,
            novel_id,
            Some(reader_a),
            1,
            2,
        )
        .await
        .unwrap();
    assert_eq!(character_a_latest.len(), 2);
    assert_eq!(character_a_latest[0].role, "user");
    assert_eq!(character_a_latest[1].role, "character");
    assert_eq!(character_a_latest[0].turn_id, character_a_latest[1].turn_id);

    let history_page = chat_repo
        .find_by_character_user(
            conversation_character_id,
            user_id,
            novel_id,
            Some(reader_a),
            1,
            2,
            0,
        )
        .await
        .unwrap();
    let repeated_page = chat_repo
        .find_by_character_user(
            conversation_character_id,
            user_id,
            novel_id,
            Some(reader_a),
            1,
            2,
            0,
        )
        .await
        .unwrap();
    let second_page = chat_repo
        .find_by_character_user(
            conversation_character_id,
            user_id,
            novel_id,
            Some(reader_a),
            1,
            2,
            2,
        )
        .await
        .unwrap();
    assert_eq!(history_page.len(), 2);
    assert_eq!(second_page.len(), 2);
    assert_eq!(
        history_page
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>(),
        repeated_page
            .iter()
            .map(|message| message.id)
            .collect::<Vec<_>>()
    );
    assert!(history_page
        .iter()
        .all(|message| second_page.iter().all(|other| other.id != message.id)));
    assert_eq!(history_page[0].role, "character");
    assert_eq!(history_page[1].role, "user");
    assert_eq!(history_page[0].turn_id, history_page[1].turn_id);
    assert_eq!(
        history_page
            .iter()
            .rev()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>(),
        vec!["user", "character"]
    );
    assert_eq!(second_page[0].role, "character");
    assert_eq!(second_page[1].role, "user");
    assert_eq!(second_page[0].turn_id, second_page[1].turn_id);

    assert_eq!(
        chat_repo
            .count(conversation_character_id, user_id, novel_id, Some(reader_b))
            .await
            .unwrap(),
        2
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn production_repositories_match_fresh_schema() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();

    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let chapter_id = Uuid::new_v4();
    let character_id = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("repository-contract-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO novels (id, user_id, title) VALUES ($1, $2, $3)")
        .bind(novel_id)
        .bind(user_id)
        .bind("Repository contract")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO chapters (id, novel_id, chapter_number, content) VALUES ($1, $2, 1, $3)",
    )
    .bind(chapter_id)
    .bind(novel_id)
    .bind("A chapter long enough for persistence testing.")
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO characters (id, novel_id, name) VALUES ($1, $2, $3)")
        .bind(character_id)
        .bind(novel_id)
        .bind("Contract Character")
        .execute(&pool)
        .await
        .unwrap();

    let memory_repo = PgMemoryRepository::new(pool.clone());
    let mut memory = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "The reader kept a promise.".into(),
        8,
        1,
    );
    let embedding = vec![0.1_f32; 1536];
    memory.embedding = Some(embedding.clone());
    memory_repo.save(&memory).await.unwrap();
    let memory_without_embedding = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "The reader chose the narrow pass.".into(),
        8,
        1,
    );
    let competing_memory_repo = PgMemoryRepository::new(pool.clone());
    let (first_insert, second_insert) = tokio::join!(
        memory_repo.insert_if_absent(&memory_without_embedding),
        competing_memory_repo.insert_if_absent(&memory_without_embedding),
    );
    assert_eq!(
        [first_insert.unwrap(), second_insert.unwrap()]
            .into_iter()
            .filter(|inserted| *inserted)
            .count(),
        1,
        "the database must elect exactly one permanent-fact writer",
    );
    assert!(!memory_repo
        .insert_if_absent(&memory_without_embedding)
        .await
        .unwrap());
    let mut future_memory = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "A future revelation.".into(),
        9,
        3,
    );
    future_memory.embedding = Some(embedding.clone());
    memory_repo.save(&future_memory).await.unwrap();
    let mut unscoped_memory = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "Unknown provenance.".into(),
        10,
        1,
    );
    unscoped_memory.chapter_number = None;
    assert!(memory_repo.save(&unscoped_memory).await.is_err());
    sqlx::query(
        "INSERT INTO character_memories (id, character_id, user_id, novel_id, layer, content, importance, chapter_number) VALUES ($1, $2, $3, $4, 'permanent', $5, 10, NULL)",
    )
    .bind(unscoped_memory.id)
    .bind(character_id)
    .bind(user_id)
    .bind(novel_id)
    .bind("Legacy memory without provenance")
    .execute(&pool)
    .await
    .unwrap();
    // A fresh adapter instance models an agent-service restart. Permanent facts
    // remain directly retrievable without an embedding; pgvector is only the
    // optional semantic-search projection.
    let restarted_memory_repo = PgMemoryRepository::new(pool.clone());
    let memories = restarted_memory_repo
        .find_by_layer(
            character_id,
            user_id,
            novel_id,
            MemoryLayer::Permanent,
            1,
            10,
            0,
        )
        .await
        .unwrap();
    assert_eq!(memories.len(), 2);
    assert!(memories
        .iter()
        .any(|memory| memory.content == "The reader chose the narrow pass."));
    assert!(memories
        .iter()
        .all(|memory| memory.content != "A future revelation."));
    assert!(restarted_memory_repo
        .find_by_layer(
            character_id,
            Uuid::new_v4(),
            novel_id,
            MemoryLayer::Permanent,
            1,
            10,
            0,
        )
        .await
        .unwrap()
        .is_empty());
    let mut quarantined_precontract = Memory::new_permanent(
        character_id,
        user_id,
        novel_id,
        "private pre-contract semantic marker".into(),
        7,
        1,
    );
    quarantined_precontract.embedding = Some(embedding.clone());
    restarted_memory_repo
        .save(&quarantined_precontract)
        .await
        .unwrap();
    let semantic = restarted_memory_repo
        .search_similar(character_id, user_id, novel_id, &embedding, 1, 5)
        .await
        .unwrap();
    assert_eq!(semantic.len(), 1);
    assert!(semantic
        .iter()
        .all(|memory| memory.id != quarantined_precontract.id));

    let chat_repo = PgChatRepository::new(pool.clone());
    let claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        user_id,
        character_id,
        novel_id,
        request_fingerprint: vec![1; 32],
        chapter_context: 1,
        reader_identity: Some("Reader".into()),
        reader_identity_type: "self".into(),
        reader_character_id: None,
        deviation_mode: "canon".into(),
    };
    let attempt = match chat_repo.begin_turn(&claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected turn reservation: {result:?}"),
    };
    let user_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "Hello".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(claim.id);
    let character_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "Welcome".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(claim.id);
    chat_repo
        .complete_turn(&claim, attempt, &user_message, &character_message)
        .await
        .unwrap();

    let future_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![2; 32],
        chapter_context: 3,
        ..claim.clone()
    };
    let future_attempt = match chat_repo.begin_turn(&future_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected future turn reservation: {result:?}"),
    };
    let future_user_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "Ask about the future".into(),
        Some("Reader".into()),
        Some(3),
    )
    .with_turn_id(future_claim.id);
    let future_character_message = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "A future spoiler".into(),
        Some("Reader".into()),
        Some(3),
    )
    .with_turn_id(future_claim.id);
    chat_repo
        .complete_turn(
            &future_claim,
            future_attempt,
            &future_user_message,
            &future_character_message,
        )
        .await
        .unwrap();
    let messages = chat_repo
        .find_recent(character_id, user_id, novel_id, None, 1, 10)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].reader_identity.as_deref(), Some("Reader"));
    assert_eq!(messages[1].chapter_context, Some(1));
    assert_eq!(
        chat_repo
            .find_by_character_user(character_id, user_id, novel_id, None, 1, 10, 0)
            .await
            .unwrap()
            .len(),
        2
    );
    let advanced_context = ChatTurnClaim {
        chapter_context: 2,
        deviation_mode: "creative".into(),
        ..claim.clone()
    };
    assert!(matches!(
        chat_repo.begin_turn(&advanced_context).await.unwrap(),
        BeginChatTurn::Completed {
            claim: persisted,
            response
        } if persisted == claim && response == "Welcome"
    ));
    assert!(matches!(
        chat_repo
            .begin_turn(&ChatTurnClaim {
                request_fingerprint: vec![9; 32],
                ..claim.clone()
            })
            .await
            .unwrap(),
        BeginChatTurn::Conflict
    ));

    let reclaim_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![3; 32],
        ..claim.clone()
    };
    let (first, second) = tokio::join!(
        chat_repo.begin_turn(&reclaim_claim),
        chat_repo.begin_turn(&reclaim_claim)
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert!(matches!(
        (&first, &second),
        (
            BeginChatTurn::Acquired { attempt: 1, .. },
            BeginChatTurn::InProgress { .. }
        ) | (
            BeginChatTurn::InProgress { .. },
            BeginChatTurn::Acquired { attempt: 1, .. }
        )
    ));
    sqlx::query(
        "UPDATE chat_turns SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(reclaim_claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let reclaimed_attempt = match chat_repo.begin_turn(&reclaim_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reclaimed turn: {result:?}"),
    };
    assert_eq!(reclaimed_attempt, 2);
    let reclaimed_user = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "Retry me".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(reclaim_claim.id);
    let reclaimed_character = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "Recovered once".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(reclaim_claim.id);
    assert!(chat_repo
        .complete_turn(&reclaim_claim, 1, &reclaimed_user, &reclaimed_character,)
        .await
        .is_err());
    chat_repo
        .complete_turn(
            &reclaim_claim,
            reclaimed_attempt,
            &reclaimed_user,
            &reclaimed_character,
        )
        .await
        .unwrap();
    let reclaimed_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM chat_messages WHERE turn_id = $1")
            .bind(reclaim_claim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(reclaimed_count, 2);

    let active_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![5; 32],
        ..claim.clone()
    };
    assert!(matches!(
        chat_repo.begin_turn(&active_claim).await.unwrap(),
        BeginChatTurn::Acquired { attempt: 1, .. }
    ));
    let next_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![6; 32],
        ..claim.clone()
    };
    assert!(matches!(
        chat_repo.begin_turn(&next_claim).await.unwrap(),
        BeginChatTurn::InProgress {
            retry_after_seconds: 1..=120
        }
    ));
    sqlx::query(
        "UPDATE chat_turns SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(active_claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let next_attempt = match chat_repo.begin_turn(&next_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected next turn reservation: {result:?}"),
    };
    assert_eq!(next_attempt, 1);
    assert!(matches!(
        chat_repo.begin_turn(&active_claim).await.unwrap(),
        BeginChatTurn::Conflict
    ));
    assert!(chat_repo
        .fail_turn(next_claim.id, next_attempt, "test_cleanup")
        .await
        .unwrap());

    let failed_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![7; 32],
        ..claim.clone()
    };
    let failed_attempt = match chat_repo.begin_turn(&failed_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected failed turn reservation: {result:?}"),
    };
    assert!(chat_repo
        .fail_turn(failed_claim.id, failed_attempt, "llm_error")
        .await
        .unwrap());
    let expired_other_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![8; 32],
        ..claim.clone()
    };
    assert!(matches!(
        chat_repo.begin_turn(&expired_other_claim).await.unwrap(),
        BeginChatTurn::Acquired { attempt: 1, .. }
    ));
    sqlx::query(
        "UPDATE chat_turns SET lease_expires_at = NOW() - INTERVAL '1 second' WHERE id = $1",
    )
    .bind(expired_other_claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let recovered_failed_attempt = match chat_repo.begin_turn(&failed_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("failed key was not recovered: {result:?}"),
    };
    assert_eq!(recovered_failed_attempt, 2);
    let (expired_status, expired_failure): (String, Option<String>) =
        sqlx::query_as("SELECT status, failure_code FROM chat_turns WHERE id = $1")
            .bind(expired_other_claim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(expired_status, "failed");
    assert_eq!(expired_failure.as_deref(), Some("superseded"));
    assert!(chat_repo
        .fail_turn(failed_claim.id, recovered_failed_attempt, "test_cleanup")
        .await
        .unwrap());

    let rollback_claim = ChatTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![4; 32],
        ..claim.clone()
    };
    let rollback_attempt = match chat_repo.begin_turn(&rollback_claim).await.unwrap() {
        BeginChatTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected rollback turn reservation: {result:?}"),
    };
    let rollback_user = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "user".into(),
        "Atomic?".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(rollback_claim.id);
    let mut rollback_character = ChatMessage::new(
        user_id,
        character_id,
        novel_id,
        "character".into(),
        "Yes".into(),
        Some("Reader".into()),
        Some(1),
    )
    .with_turn_id(rollback_claim.id);
    rollback_character.id = rollback_user.id;
    assert!(chat_repo
        .complete_turn(
            &rollback_claim,
            rollback_attempt,
            &rollback_user,
            &rollback_character,
        )
        .await
        .is_err());
    let (rollback_status, rollback_messages): (String, i64) = sqlx::query_as(
        "SELECT status, (SELECT COUNT(*) FROM chat_messages WHERE turn_id = $1) FROM chat_turns WHERE id = $1",
    )
    .bind(rollback_claim.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(rollback_status, "in_progress");
    assert_eq!(rollback_messages, 0);

    let world_state_repo = PgWorldStateRepository::new(pool.clone());
    let mut legacy_state = world_state_repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap();
    let player = PlayerEntity::new(
        user_id,
        novel_id,
        5,
        "云舟".into(),
        "来自边城的地图学徒。".into(),
        vec!["辨认古地图".into()],
        "north-tower".into(),
        vec!["旧地图".into()],
    )
    .unwrap();
    legacy_state.state["relationships"] = serde_json::json!({
        "not-a-uuid": {"score": 55, "last_change": "malformed legacy state"}
    });
    world_state_repo.update(&legacy_state).await.unwrap();
    let malformed_before: (serde_json::Value, String) = sqlx::query_as(
        "SELECT state, updated_at::text FROM world_states WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(world_state_repo
        .create_player_entity(&player)
        .await
        .is_err());
    let malformed_after: (serde_json::Value, String) = sqlx::query_as(
        "SELECT state, updated_at::text FROM world_states WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(malformed_after, malformed_before);

    legacy_state.state["relationships"] = serde_json::json!({});
    legacy_state.state["choices"] = serde_json::json!([{ "chapter": 6 }]);
    world_state_repo.update(&legacy_state).await.unwrap();
    let future_choice_before: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(world_state_repo
        .create_player_entity(&player)
        .await
        .is_err());
    let future_choice_after: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(future_choice_after, future_choice_before);
    assert!(future_choice_after.get("player_entity").is_none());

    legacy_state.state["choices"] = serde_json::json!([]);
    legacy_state
        .update_relationship(&character_id.to_string(), 5, "legacy trust")
        .unwrap();
    world_state_repo.update(&legacy_state).await.unwrap();
    let (left_player, right_player) = tokio::join!(
        world_state_repo.create_player_entity(&player),
        world_state_repo.create_player_entity(&player)
    );
    assert_eq!(left_player.unwrap(), right_player.unwrap());
    let competing = PlayerEntity::new(
        user_id,
        novel_id,
        1,
        "另一名玩家".into(),
        "来自另一条时间线。".into(),
        vec!["观察".into()],
        "north-tower".into(),
        vec![],
    )
    .unwrap();
    let competing_error = world_state_repo
        .create_player_entity(&competing)
        .await
        .unwrap_err();
    assert!(matches!(
        competing_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(_))
    ));

    let node_repo = PgNarrativeNodeRepository::new(pool.clone());
    let node = NarrativeNode::new(
        novel_id,
        1,
        "A decisive moment".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "Stay".into(),
            hint: "Stand your ground".into(),
            generated_consequence: None,
        }],
    );
    node_repo.save(&node).await.unwrap();
    node_repo.save(&node).await.unwrap();
    assert_eq!(
        node_repo
            .find_by_chapter(novel_id, 1, None)
            .await
            .unwrap()
            .unwrap()
            .id,
        node.id
    );

    let choice_repo = PgUserChoiceRepository::new(pool.clone());
    sqlx::query(
        "INSERT INTO player_chapters (user_id, novel_id, chapter_number, content, origin) \
         VALUES ($1, $2, 1, '旧的续写章节。', 'continuation')",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let initial_choice_fingerprint = world_state_repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap()
        .fingerprint();
    let mut first_transition = transition(1, "角色决定留下。");
    first_transition
        .relationship_changes
        .push(RelationshipChange {
            character_id,
            delta: 10,
            reason: "kept a promise".into(),
        });
    let draft = ChoiceCommit {
        user_id,
        novel_id,
        node_id: node.id,
        chapter_number: 1,
        choice_index: 0,
        choice_text: "Stay".into(),
        expected_world_state_fingerprint: initial_choice_fingerprint,
        transition: first_transition,
        rewritten_chapter_content: "原著开篇。角色决定留下。".into(),
    };
    let (left, right) = tokio::join!(
        choice_repo.commit_choice(&draft),
        choice_repo.commit_choice(&draft)
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert_eq!(left.choice.id, right.choice.id);
    assert_eq!(left.player_chapter_content, "原著开篇。角色决定留下。");
    assert_eq!(left.player_chapter_content, right.player_chapter_content);
    let persisted_player_chapter: (String, String) = sqlx::query_as(
        "SELECT content, origin FROM player_chapters \
         WHERE user_id = $1 AND novel_id = $2 AND chapter_number = 1",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        persisted_player_chapter,
        ("原著开篇。角色决定留下。".into(), "choice".into())
    );
    sqlx::query(
        "UPDATE player_chapters SET content = '遗留续写正文。', origin = 'continuation' \
         WHERE user_id = $1 AND novel_id = $2 AND chapter_number = 1",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let legacy_replay = choice_repo.commit_choice(&draft).await.unwrap();
    assert!(legacy_replay
        .player_chapter_content
        .contains(&left.choice.consequence));
    assert_eq!(
        sqlx::query_as::<_, (String, String)>(
            "SELECT content, origin FROM player_chapters \
             WHERE user_id = $1 AND novel_id = $2 AND chapter_number = 1",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        (legacy_replay.player_chapter_content, "choice".into())
    );
    assert_eq!(
        left.world_state.state["choices"].as_array().unwrap().len(),
        1
    );
    let persisted_choices: serde_json::Value = sqlx::query_scalar(
        "SELECT state -> 'choices' FROM world_states WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted_choices.as_array().unwrap().len(), 1);
    let persisted_state: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(persisted_state.get("relationships").is_none());
    assert_eq!(
        persisted_state["player_entity"]["relationships"][character_id.to_string()]["score"],
        65
    );
    assert_eq!(
        choice_repo
            .find_user_choice(user_id, node.id)
            .await
            .unwrap()
            .unwrap()
            .choice_index,
        0
    );

    let choice_prefix_fingerprint = world_state_repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap()
        .fingerprint();
    let mut more_drafts = Vec::new();
    for chapter_number in 2..=4 {
        let next = NarrativeNode::new(
            novel_id,
            chapter_number,
            format!("Decision {chapter_number}"),
            vec![NarrativeChoice {
                index: 0,
                text: format!("Choice {chapter_number}"),
                hint: "Continue".into(),
                generated_consequence: None,
            }],
        );
        node_repo.save(&next).await.unwrap();
        more_drafts.push(ChoiceCommit {
            user_id,
            novel_id,
            node_id: next.id,
            chapter_number,
            choice_index: 0,
            choice_text: format!("Choice {chapter_number}"),
            expected_world_state_fingerprint: choice_prefix_fingerprint,
            transition: transition(
                chapter_number,
                format!("第{chapter_number}章的选择已经生效。"),
            ),
            rewritten_chapter_content: format!("Rewritten chapter {chapter_number}"),
        });
    }
    let before_player_chapter_conflict: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    sqlx::query(
        "INSERT INTO player_chapters (user_id, novel_id, chapter_number, content, origin) \
         VALUES ($1, $2, 2, '另一条已提交分支。', 'choice')",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let player_chapter_conflict = choice_repo
        .commit_choice(&more_drafts[0])
        .await
        .unwrap_err();
    assert!(player_chapter_conflict
        .to_string()
        .contains("player chapter conflicts with the committed choice"));
    assert!(matches!(
        player_chapter_conflict.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2",
        )
        .bind(user_id)
        .bind(more_drafts[0].node_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    let after_player_chapter_conflict: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        after_player_chapter_conflict,
        before_player_chapter_conflict
    );
    assert_eq!(
        sqlx::query_as::<_, (String, String)>(
            "SELECT content, origin FROM player_chapters \
             WHERE user_id = $1 AND novel_id = $2 AND chapter_number = 2",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        ("另一条已提交分支。".into(), "choice".into())
    );
    sqlx::query(
        "DELETE FROM player_chapters \
         WHERE user_id = $1 AND novel_id = $2 AND chapter_number = 2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let second = choice_repo.commit_choice(&more_drafts[0]).await.unwrap();
    let stale_third = choice_repo
        .commit_choice(&more_drafts[1])
        .await
        .unwrap_err();
    assert!(matches!(
        stale_third.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2"
        )
        .bind(user_id)
        .bind(more_drafts[1].node_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM player_chapters \
             WHERE user_id = $1 AND novel_id = $2 AND chapter_number = 3"
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    let replayed_second = choice_repo.commit_choice(&more_drafts[0]).await.unwrap();
    assert_eq!(replayed_second.choice.id, second.choice.id);
    assert_eq!(
        replayed_second.player_chapter_content,
        second.player_chapter_content
    );
    more_drafts[1].expected_world_state_fingerprint = world_state_repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap()
        .fingerprint();
    choice_repo.commit_choice(&more_drafts[1]).await.unwrap();
    let valid_state: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(valid_state["choices"].as_array().unwrap().len(), 3);

    let too_late = NarrativeNode::new(
        novel_id,
        6,
        "Decision 6".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "Choice 6".into(),
            hint: "Too late".into(),
            generated_consequence: None,
        }],
    );
    node_repo.save(&too_late).await.unwrap();
    let too_late_draft = ChoiceCommit {
        user_id,
        novel_id,
        node_id: too_late.id,
        chapter_number: 6,
        choice_index: 0,
        choice_text: "Choice 6".into(),
        expected_world_state_fingerprint: WorldState {
            user_id,
            novel_id,
            state: valid_state.clone(),
            updated_at: chrono::Utc::now(),
        }
        .fingerprint(),
        transition: transition(6, "这条选择不应越过玩家锚点。"),
        rewritten_chapter_content: "Rejected chapter 6".into(),
    };
    assert!(choice_repo.commit_choice(&too_late_draft).await.is_err());
    assert!(choice_repo
        .find_user_choice(user_id, too_late.id)
        .await
        .unwrap()
        .is_none());
    let after_rejected_choice: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(after_rejected_choice, valid_state);

    sqlx::query(
        "UPDATE world_states SET state = jsonb_set(state, '{player_entity,unknown}', 'true'::jsonb) WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let malformed_player_state: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    more_drafts[2].expected_world_state_fingerprint = WorldState {
        user_id,
        novel_id,
        state: malformed_player_state,
        updated_at: chrono::Utc::now(),
    }
    .fingerprint();
    let malformed_player_error = choice_repo
        .commit_choice(&more_drafts[2])
        .await
        .unwrap_err();
    assert!(matches!(
        malformed_player_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::InvalidPlayerEntity(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2",
        )
        .bind(user_id)
        .bind(more_drafts[2].node_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    sqlx::query("UPDATE world_states SET state = $3 WHERE user_id = $1 AND novel_id = $2")
        .bind(user_id)
        .bind(novel_id)
        .bind(&valid_state)
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query(
        "UPDATE world_states SET state = '{\"choices\":{}}'::jsonb WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let malformed_choices_state: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    more_drafts[2].expected_world_state_fingerprint = WorldState {
        user_id,
        novel_id,
        state: malformed_choices_state,
        updated_at: chrono::Utc::now(),
    }
    .fingerprint();
    let malformed_choices_error = choice_repo
        .commit_choice(&more_drafts[2])
        .await
        .unwrap_err();
    assert!(matches!(
        malformed_choices_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(message))
            if message.contains("durable branch choices")
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2",
        )
        .bind(user_id)
        .bind(more_drafts[2].node_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );
    sqlx::query("UPDATE world_states SET state = $3 WHERE user_id = $1 AND novel_id = $2")
        .bind(user_id)
        .bind(novel_id)
        .bind(valid_state)
        .execute(&pool)
        .await
        .unwrap();
    more_drafts[2].expected_world_state_fingerprint = world_state_repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap()
        .fingerprint();
    choice_repo.commit_choice(&more_drafts[2]).await.unwrap();

    let non_monotonic_node = NarrativeNode::new(
        novel_id,
        4,
        "A second decision at an already committed chapter".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "Rewrite the past".into(),
            hint: "Rejected".into(),
            generated_consequence: None,
        }],
    )
    .for_user(user_id);
    node_repo.save(&non_monotonic_node).await.unwrap();
    let non_monotonic_draft = ChoiceCommit {
        user_id,
        novel_id,
        node_id: non_monotonic_node.id,
        chapter_number: 4,
        choice_index: 0,
        choice_text: "Rewrite the past".into(),
        expected_world_state_fingerprint: world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .unwrap()
            .fingerprint(),
        transition: transition(4, "这条同章选择不应进入已提交前缀。"),
        rewritten_chapter_content: "Rejected same-chapter rewrite".into(),
    };
    let non_monotonic_error = choice_repo
        .commit_choice(&non_monotonic_draft)
        .await
        .unwrap_err();
    assert!(matches!(
        non_monotonic_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(_))
    ));
    assert!(choice_repo
        .find_user_choice(user_id, non_monotonic_node.id)
        .await
        .unwrap()
        .is_none());

    let competing_node = NarrativeNode::new(
        novel_id,
        5,
        "Competing decision".into(),
        vec![
            NarrativeChoice {
                index: 0,
                text: "Left".into(),
                hint: "Take the left path".into(),
                generated_consequence: None,
            },
            NarrativeChoice {
                index: 1,
                text: "Right".into(),
                hint: "Take the right path".into(),
                generated_consequence: None,
            },
        ],
    );
    node_repo.save(&competing_node).await.unwrap();
    let left_draft = ChoiceCommit {
        user_id,
        novel_id,
        node_id: competing_node.id,
        chapter_number: 5,
        choice_index: 0,
        choice_text: "Left".into(),
        expected_world_state_fingerprint: world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .unwrap()
            .fingerprint(),
        transition: transition(5, "角色选择左路。"),
        rewritten_chapter_content: "原著开篇。角色选择左路。".into(),
    };
    let right_draft = ChoiceCommit {
        choice_index: 1,
        choice_text: "Right".into(),
        transition: transition(5, "角色选择右路。"),
        rewritten_chapter_content: "原著开篇。角色选择右路。".into(),
        ..left_draft.clone()
    };
    let (left_result, right_result) = tokio::join!(
        choice_repo.commit_choice(&left_draft),
        choice_repo.commit_choice(&right_draft)
    );
    let (committed, rejected) = match (left_result, right_result) {
        (Ok(committed), Err(rejected)) | (Err(rejected), Ok(committed)) => (committed, rejected),
        outcomes => panic!("expected one committed choice and one rejection, got {outcomes:?}"),
    };
    assert!(rejected
        .to_string()
        .contains("player chapter conflicts with the committed choice"));
    assert!(matches!(
        rejected.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(_))
    ));
    match committed.choice.choice_index {
        0 => assert_eq!(committed.player_chapter_content, "原著开篇。角色选择左路。"),
        1 => assert_eq!(committed.player_chapter_content, "原著开篇。角色选择右路。"),
        index => panic!("unexpected competing choice index {index}"),
    }
    let competing_choice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2")
            .bind(user_id)
            .bind(competing_node.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(competing_choice_count, 1);

    let world_context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 5,
        unlocked_through_chapter: 6,
        characters: vec![WorldCharacterRef {
            id: character_id,
            name: "Hero".into(),
        }],
        locations: vec![WorldEntityRef {
            id: "north-tower".into(),
            name: "North Tower".into(),
        }],
        factions: vec![WorldEntityRef {
            id: "wardens".into(),
            name: "Wardens".into(),
        }],
        hard_rules: vec![WorldRuleRef {
            id: "rule-1".into(),
            description: "The dead remain dead.".into(),
        }],
        dead_character_ids: vec![],
        threads: vec![WorldEntityRef {
            id: "thread-1".into(),
            name: "Find the hidden path".into(),
        }],
        scheduled_events: vec![ScheduledCanonEvent {
            id: "event-2".into(),
            sequence: 2,
            summary: "The wardens defend the tower.".into(),
            character_ids: vec![character_id],
            location_ids: vec!["north-tower".into()],
            faction_ids: vec!["wardens".into()],
            death_character_ids: vec![],
            source_chapters: vec![6],
        }],
        character_goals: vec![CharacterGoalRef {
            id: "goal-1".into(),
            character_id,
            description: "Protect the tower.".into(),
            source_chapters: vec![1],
        }],
    };
    let started = world_state_repo
        .start_open_world(user_id, novel_id, &world_context, None)
        .await
        .unwrap();
    assert_eq!(started.open_world().unwrap().unwrap().turn_number, 0);
    let mut drifted_context = world_context.clone();
    drifted_context.model_version = 2;
    let resumed = world_state_repo
        .start_open_world(user_id, novel_id, &drifted_context, None)
        .await
        .unwrap();
    assert_eq!(resumed, started);
    assert_eq!(
        resumed
            .open_world()
            .unwrap()
            .unwrap()
            .entry_context
            .model_version,
        1
    );

    let world_turn_repo = PgWorldTurnRepository::new(pool.clone());
    let action = WorldAction {
        kind: WorldActionKind::Investigate,
        target_id: Some("thread-1".into()),
        intent: "追查塔中的隐秘道路".into(),
    };
    let claim = WorldTurnClaim {
        id: Uuid::new_v4(),
        user_id,
        novel_id,
        request_fingerprint: vec![7; 32],
        action: action.clone(),
        expected_turn_number: 0,
        resolution: None,
    };
    let attempt = match world_turn_repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected world turn reservation: {result:?}"),
    };
    let competing = WorldTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![8; 32],
        ..claim.clone()
    };
    assert!(matches!(
        world_turn_repo.begin_turn(&competing).await.unwrap(),
        BeginWorldTurn::InProgress { .. }
    ));
    let world_transition = WorldTurnTransition {
        schema_version: WORLD_TURN_SCHEMA_VERSION,
        prompt_version: WORLD_TURN_PROMPT_VERSION.into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: 5,
        rendered_narrative: "你在塔中找到一条隐秘道路，守门人开始相信你的判断。".into(),
        events: vec![TransitionEvent {
            summary: "玩家找到隐秘道路".into(),
            actor_character_ids: vec![],
            location_id: Some("north-tower".into()),
        }],
        relationship_changes: vec![RelationshipChange {
            character_id,
            delta: 5,
            reason: "共享隐秘道路".into(),
        }],
        location_changes: vec![],
        thread_changes: vec![],
        player_location_id: None,
        inventory_additions: vec!["隐秘地图".into()],
        inventory_removals: vec![],
        knowledge_discoveries: vec!["北塔有隐秘道路".into()],
        faction_changes: vec![FactionStandingChange {
            faction_id: "wardens".into(),
            delta: 5,
            reason: "帮助守军".into(),
        }],
        canonical_event_change: None,
    };
    let completed = world_turn_repo
        .complete_turn(&claim, attempt, &world_transition, &world_context)
        .await
        .unwrap();
    assert_eq!(
        completed
            .world_state
            .open_world()
            .unwrap()
            .unwrap()
            .turn_number,
        1
    );
    assert_eq!(
        completed
            .world_state
            .player_entity()
            .unwrap()
            .unwrap()
            .inventory,
        vec!["旧地图", "隐秘地图"]
    );
    assert_eq!(
        world_state_repo
            .get_or_create(user_id, novel_id)
            .await
            .unwrap(),
        completed.world_state
    );
    match world_turn_repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Completed {
            result: replayed,
            memory_projection,
        } => {
            assert_eq!(*replayed, completed);
            assert_eq!(memory_projection, MemoryProjectionStatus::Pending);
        }
        result => panic!("completed world turn did not replay: {result:?}"),
    }
    assert!(world_turn_repo
        .finish_memory_projection(claim.id, user_id, novel_id, MemoryProjectionStatus::Saved,)
        .await
        .unwrap());
    let advanced_replay = WorldTurnClaim {
        expected_turn_number: 1,
        ..claim.clone()
    };
    match world_turn_repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Completed {
            result: replayed,
            memory_projection,
        } => {
            assert_eq!(*replayed, completed);
            assert_eq!(memory_projection, MemoryProjectionStatus::Saved);
        }
        result => panic!("exact completed world turn did not replay: {result:?}"),
    }
    assert!(matches!(
        world_turn_repo.begin_turn(&advanced_replay).await.unwrap(),
        BeginWorldTurn::Conflict
    ));
    assert!(!world_turn_repo
        .finish_memory_projection(claim.id, user_id, novel_id, MemoryProjectionStatus::Skipped,)
        .await
        .unwrap());
    let conflicting_reuse = WorldTurnClaim {
        action: WorldAction {
            intent: "不同动作".into(),
            ..action.clone()
        },
        ..claim.clone()
    };
    assert!(matches!(
        world_turn_repo
            .begin_turn(&conflicting_reuse)
            .await
            .unwrap(),
        BeginWorldTurn::Conflict
    ));
    assert!(matches!(
        world_turn_repo.begin_turn(&competing).await.unwrap(),
        BeginWorldTurn::Stale
    ));
    let current_journal = world_turn_repo
        .journal(user_id, novel_id, 100)
        .await
        .unwrap();
    assert_eq!(current_journal.len(), 1);
    assert_eq!(
        current_journal[0].transition.prompt_version,
        WORLD_TURN_PROMPT_VERSION
    );
    assert_eq!(
        current_journal[0].transition.rendered_narrative,
        "你在塔中找到一条隐秘道路，守门人开始相信你的判断。"
    );

    let rollback_claim = WorldTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![9; 32],
        expected_turn_number: 1,
        ..claim.clone()
    };
    let rollback_attempt = match world_turn_repo.begin_turn(&rollback_claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected rollback reservation: {result:?}"),
    };
    let rollback_transition = WorldTurnTransition {
        inventory_additions: vec![],
        inventory_removals: vec!["不存在的物品".into()],
        knowledge_discoveries: vec![],
        faction_changes: vec![],
        relationship_changes: vec![],
        rendered_narrative: "你试图使用一件并不存在的物品，世界拒绝了这次变化。".into(),
        events: vec![TransitionEvent {
            summary: "无效物品操作".into(),
            actor_character_ids: vec![],
            location_id: Some("north-tower".into()),
        }],
        ..world_transition.clone()
    };
    assert!(world_turn_repo
        .complete_turn(
            &rollback_claim,
            rollback_attempt,
            &rollback_transition,
            &world_context,
        )
        .await
        .is_err());
    let (persisted_turn, rollback_status): (i64, String) = sqlx::query_as(
        "SELECT (state #>> '{open_world,turn_number}')::BIGINT, \
                (SELECT status FROM world_turns WHERE id = $3) \
         FROM world_states WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .bind(rollback_claim.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(persisted_turn, 1);
    assert_eq!(rollback_status, "in_progress");
    assert!(world_turn_repo
        .fail_turn(rollback_claim.id, rollback_attempt, "test_cleanup")
        .await
        .unwrap());

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

// ─── H4: world-turn idempotency and race contracts on real PostgreSQL ───
//
// These tests prove zero duplicate commit across retry/reordering races and
// completed-key replay semantics at the persistence layer (H4 exit evidence).

#[tokio::test]
async fn player_checkpoint_race_elects_exactly_one_timeline() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("player-race-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status) \
         VALUES ($1, $2, $3, 2, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind("Player checkpoint race")
    .execute(&pool)
    .await
    .unwrap();
    let repo = PgWorldStateRepository::new(pool.clone());
    repo.get_or_create(user_id, novel_id).await.unwrap();
    let at_chapter_one = PlayerEntity::new(
        user_id,
        novel_id,
        1,
        "云舟".into(),
        "来自边城的地图学徒。".into(),
        vec!["辨认古地图".into()],
        "north-tower".into(),
        vec!["旧地图".into()],
    )
    .unwrap();
    let at_chapter_two = PlayerEntity::new(
        user_id,
        novel_id,
        2,
        at_chapter_one.name.clone(),
        at_chapter_one.background.clone(),
        at_chapter_one.capabilities.clone(),
        at_chapter_one.location_id.clone(),
        at_chapter_one.inventory.clone(),
    )
    .unwrap();

    let (left, right) = tokio::join!(
        repo.create_player_entity(&at_chapter_one),
        repo.create_player_entity(&at_chapter_two)
    );
    let results = [left, right];
    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let error = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .expect("one checkpoint must lose the race");
    assert!(matches!(
        error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(_))
    ));
    let stored = repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap()
        .player_entity()
        .unwrap()
        .unwrap();
    assert!([1, 2].contains(&stored.canonical_checkpoint_chapter));

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn open_world_and_choice_race_linearizes_without_partial_commit() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("choice-world-race-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status) \
         VALUES ($1, $2, $3, 1, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind("Choice and world race")
    .execute(&pool)
    .await
    .unwrap();

    let world_repo = PgWorldStateRepository::new(pool.clone());
    world_repo.get_or_create(user_id, novel_id).await.unwrap();
    let player = PlayerEntity::new(
        user_id,
        novel_id,
        1,
        "云舟".into(),
        "来自边城的地图学徒。".into(),
        vec!["辨认古地图".into()],
        "north-tower".into(),
        vec![],
    )
    .unwrap();
    world_repo.create_player_entity(&player).await.unwrap();
    let context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 1,
        unlocked_through_chapter: 1,
        characters: vec![],
        locations: vec![WorldEntityRef {
            id: "north-tower".into(),
            name: "North Tower".into(),
        }],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![],
        character_goals: vec![],
    };
    let node_repo = PgNarrativeNodeRepository::new(pool.clone());
    let choice_repo = PgUserChoiceRepository::new(pool.clone());
    let node = NarrativeNode::new(
        novel_id,
        1,
        "The last branch before entry".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "Enter".into(),
            hint: "Continue".into(),
            generated_consequence: None,
        }],
    );
    node_repo.save(&node).await.unwrap();
    let draft = ChoiceCommit {
        user_id,
        novel_id,
        node_id: node.id,
        chapter_number: 1,
        choice_index: 0,
        choice_text: "Enter".into(),
        expected_world_state_fingerprint: world_repo
            .get_or_create(user_id, novel_id)
            .await
            .unwrap()
            .fingerprint(),
        transition: transition(1, "你在进入世界前作出了最后选择。"),
        rewritten_chapter_content: "你在进入世界前作出了最后选择。".into(),
    };

    let (choice_result, world_result) = tokio::join!(
        choice_repo.commit_choice(&draft),
        world_repo.start_open_world(user_id, novel_id, &context, None)
    );
    let started = world_result.unwrap();
    assert!(started.open_world().unwrap().is_some());
    let persisted_choice_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2")
            .bind(user_id)
            .bind(node.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let persisted_chapter_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM player_chapters \
         WHERE user_id = $1 AND novel_id = $2 AND chapter_number = 1",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    match choice_result {
        Ok(_) => {
            assert_eq!(persisted_choice_count, 1);
            assert_eq!(persisted_chapter_count, 1);
        }
        Err(error) => {
            assert!(matches!(
                error.downcast_ref::<WorldStateError>(),
                Some(WorldStateError::TimelineConflict(_))
            ));
            assert_eq!(persisted_choice_count, 0);
            assert_eq!(persisted_chapter_count, 0);
        }
    }

    let frozen_node = NarrativeNode::new(
        novel_id,
        2,
        "A branch submitted after entry".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "Too late".into(),
            hint: "Rejected".into(),
            generated_consequence: None,
        }],
    );
    node_repo.save(&frozen_node).await.unwrap();
    let frozen_draft = ChoiceCommit {
        node_id: frozen_node.id,
        chapter_number: 2,
        choice_text: "Too late".into(),
        transition: transition(2, "这条选择不应提交。"),
        rewritten_chapter_content: "这条选择不应提交。".into(),
        ..draft
    };
    let frozen_error = choice_repo.commit_choice(&frozen_draft).await.unwrap_err();
    assert!(matches!(
        frozen_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(_))
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2",
        )
        .bind(user_id)
        .bind(frozen_node.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn legacy_orphan_choice_blocks_sealed_world_boundaries_and_exact_replay() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("legacy-orphan-choice-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status) \
         VALUES ($1, $2, $3, 7, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind("Legacy orphan choice")
    .execute(&pool)
    .await
    .unwrap();

    let world_repo = PgWorldStateRepository::new(pool.clone());
    let node_repo = PgNarrativeNodeRepository::new(pool.clone());
    let choice_repo = PgUserChoiceRepository::new(pool.clone());
    let turn_repo = PgWorldTurnRepository::new(pool.clone());
    let initial = world_repo.get_or_create(user_id, novel_id).await.unwrap();
    let node = NarrativeNode::new(
        novel_id,
        7,
        "A durable late choice".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "Take the hidden road".into(),
            hint: "Commit the branch".into(),
            generated_consequence: None,
        }],
    );
    node_repo.save(&node).await.unwrap();
    let draft = ChoiceCommit {
        user_id,
        novel_id,
        node_id: node.id,
        chapter_number: 7,
        choice_index: 0,
        choice_text: "Take the hidden road".into(),
        expected_world_state_fingerprint: initial.fingerprint(),
        transition: transition(7, "你选择了隐藏道路。"),
        rewritten_chapter_content: "你选择了隐藏道路。".into(),
    };
    choice_repo.commit_choice(&draft).await.unwrap();

    // Supported legacy shape: the durable choice survived, but its JSONB
    // projection did not. No authority boundary may seal or mutate this state.
    sqlx::query(
        "UPDATE world_states SET state = jsonb_set(state, '{choices}', '[]'::jsonb) \
         WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    let corrupted_before: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let player = PlayerEntity::new(
        user_id,
        novel_id,
        1,
        "云舟".into(),
        "来自边城的地图学徒。".into(),
        vec!["辨认古地图".into()],
        "north-tower".into(),
        vec![],
    )
    .unwrap();
    let player_error = world_repo.create_player_entity(&player).await.unwrap_err();
    assert!(matches!(
        player_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(message))
            if message.contains("durable branch choices")
    ));

    let context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 1,
        unlocked_through_chapter: 1,
        characters: vec![],
        locations: vec![WorldEntityRef {
            id: "north-tower".into(),
            name: "North Tower".into(),
        }],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![],
        character_goals: vec![],
    };
    let world_error = world_repo
        .start_open_world(user_id, novel_id, &context, None)
        .await
        .unwrap_err();
    assert!(matches!(
        world_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(message))
            if message.contains("durable branch choices")
    ));

    let claim = world_turn_claim(user_id, novel_id);
    let turn_error = turn_repo.begin_turn(&claim).await.unwrap_err();
    assert!(matches!(
        turn_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(message))
            if message.contains("durable branch choices")
    ));
    let replay_error = choice_repo.commit_choice(&draft).await.unwrap_err();
    assert!(matches!(
        replay_error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(message))
            if message.contains("durable branch choices")
    ));

    let corrupted_after: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(corrupted_after, corrupted_before);
    assert!(corrupted_after["choices"].as_array().unwrap().is_empty());
    assert!(corrupted_after.get("player_entity").is_none());
    assert!(corrupted_after.get("open_world").is_none());
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM user_choices WHERE user_id = $1 AND node_id = $2",
        )
        .bind(user_id)
        .bind(node.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM world_turns WHERE user_id = $1 AND novel_id = $2",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        0
    );

    let keyed_choice = serde_json::json!({
        "node_id": node.id,
        "chapter": 7,
        "choice_index": 0,
        "choice": "Take the hidden road",
        "consequence": draft.transition.rendered_narrative,
        "canon_model_version": draft.transition.canon_model_version,
        "canonical_checkpoint_chapter": draft.transition.canonical_checkpoint_chapter,
    });
    for malformed_choices in [
        serde_json::json!([{
            "chapter": 7,
            "choice": "Take the hidden road",
        }]),
        serde_json::json!(["not-an-object"]),
        serde_json::json!([keyed_choice.clone(), keyed_choice.clone()]),
    ] {
        sqlx::query(
            "UPDATE world_states SET state = jsonb_set(state, '{choices}', $3) \
             WHERE user_id = $1 AND novel_id = $2",
        )
        .bind(user_id)
        .bind(novel_id)
        .bind(malformed_choices)
        .execute(&pool)
        .await
        .unwrap();
        let state_before: serde_json::Value = sqlx::query_scalar(
            "SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(matches!(
            world_repo
                .create_player_entity(&player)
                .await
                .unwrap_err()
                .downcast_ref::<WorldStateError>(),
            Some(WorldStateError::TimelineConflict(_))
        ));
        assert!(matches!(
            world_repo
                .start_open_world(user_id, novel_id, &context, None)
                .await
                .unwrap_err()
                .downcast_ref::<WorldStateError>(),
            Some(WorldStateError::TimelineConflict(_))
        ));
        assert!(matches!(
            turn_repo
                .begin_turn(&claim)
                .await
                .unwrap_err()
                .downcast_ref::<WorldStateError>(),
            Some(WorldStateError::TimelineConflict(_))
        ));

        let state_after: serde_json::Value = sqlx::query_scalar(
            "SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(state_after, state_before);
        assert!(state_after.get("player_entity").is_none());
        assert!(state_after.get("open_world").is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM world_turns WHERE user_id = $1 AND novel_id = $2",
            )
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
            0
        );
    }

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_complete_rechecks_durable_choice_projection() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, context) = seed_world_turn(&pool).await;
    let turn_repo = PgWorldTurnRepository::new(pool.clone());
    let claim = world_turn_claim(user_id, novel_id);
    let attempt = world_turn_acquire(&turn_repo, &claim).await;
    let state_before: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    let node = NarrativeNode::new(
        novel_id,
        7,
        "A choice lost from the projection".into(),
        vec![NarrativeChoice {
            index: 0,
            text: "Late branch".into(),
            hint: "Must not cross the turn commit".into(),
            generated_consequence: None,
        }],
    );
    PgNarrativeNodeRepository::new(pool.clone())
        .save(&node)
        .await
        .unwrap();
    let choice_transition = transition(7, "这条旧选择没有 JSONB 投影。");
    sqlx::query(
        "INSERT INTO user_choices (\
             user_id, novel_id, node_id, chapter_number, choice_index, \
             choice_text, consequence, transition\
         ) VALUES ($1, $2, $3, 7, 0, $4, $5, $6)",
    )
    .bind(user_id)
    .bind(novel_id)
    .bind(node.id)
    .bind("Late branch")
    .bind(&choice_transition.rendered_narrative)
    .bind(serde_json::to_value(&choice_transition).unwrap())
    .execute(&pool)
    .await
    .unwrap();

    let error = turn_repo
        .complete_turn(&claim, attempt, &world_turn_transition(), &context)
        .await
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<WorldStateError>(),
        Some(WorldStateError::TimelineConflict(message))
            if message.contains("durable branch choices")
    ));
    let state_after: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(state_after, state_before);
    let turn_status: (String, i64) =
        sqlx::query_as("SELECT status, attempt FROM world_turns WHERE id = $1")
            .bind(claim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(turn_status, ("in_progress".into(), attempt));
    assert!(turn_repo
        .fail_turn(claim.id, attempt, "commit_error")
        .await
        .unwrap());
    assert_eq!(
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT status, failure_code FROM world_turns WHERE id = $1",
        )
        .bind(claim.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        ("failed".into(), Some("commit_error".into()))
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

async fn seed_world_turn(pool: &PgPool) -> (Uuid, Uuid, WorldEntryContext) {
    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(format!("world-turn-{user_id}@test.invalid"))
        .bind("not-a-real-password-hash")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, total_chapters, status) \
         VALUES ($1, $2, $3, 1, 'ready')",
    )
    .bind(novel_id)
    .bind(user_id)
    .bind("World turn contract")
    .execute(pool)
    .await
    .unwrap();
    let world_state_repo = PgWorldStateRepository::new(pool.clone());
    world_state_repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap();
    let player = PlayerEntity::new(
        user_id,
        novel_id,
        1,
        "云舟".into(),
        "来自边城的地图学徒。".into(),
        vec!["辨认古地图".into()],
        "north-tower".into(),
        vec!["旧地图".into()],
    )
    .unwrap();
    world_state_repo
        .create_player_entity(&player)
        .await
        .unwrap();
    let context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 1,
        unlocked_through_chapter: 2,
        characters: vec![],
        locations: vec![
            WorldEntityRef {
                id: "north-tower".into(),
                name: "North Tower".into(),
            },
            WorldEntityRef {
                id: "harbor".into(),
                name: "Harbor".into(),
            },
        ],
        factions: vec![],
        hard_rules: vec![],
        dead_character_ids: vec![],
        threads: vec![],
        scheduled_events: vec![ScheduledCanonEvent {
            id: "siege-event".into(),
            sequence: 1,
            summary: "围城开始".into(),
            character_ids: vec![],
            location_ids: vec!["north-tower".into()],
            faction_ids: vec![],
            death_character_ids: vec![],
            source_chapters: vec![2],
        }],
        character_goals: vec![],
    };
    world_state_repo
        .start_open_world(user_id, novel_id, &context, None)
        .await
        .unwrap();
    (user_id, novel_id, context)
}

fn world_turn_transition() -> WorldTurnTransition {
    WorldTurnTransition {
        schema_version: WORLD_TURN_SCHEMA_VERSION,
        prompt_version: WORLD_TURN_PROMPT_VERSION.into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: 1,
        rendered_narrative: "你在北塔找到一条隐秘道路。".into(),
        events: vec![TransitionEvent {
            summary: "玩家找到隐秘道路".into(),
            actor_character_ids: vec![],
            location_id: Some("north-tower".into()),
        }],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
        player_location_id: Some("north-tower".into()),
        inventory_additions: vec![],
        inventory_removals: vec![],
        knowledge_discoveries: vec![],
        faction_changes: vec![],
        canonical_event_change: None,
    }
}

fn world_turn_action() -> WorldAction {
    WorldAction {
        kind: WorldActionKind::Travel,
        target_id: Some("north-tower".into()),
        intent: "前往北塔".into(),
    }
}

fn world_turn_claim(user_id: Uuid, novel_id: Uuid) -> WorldTurnClaim {
    world_turn_claim_at(user_id, novel_id, 0)
}

fn world_turn_claim_at(user_id: Uuid, novel_id: Uuid, expected_turn_number: i64) -> WorldTurnClaim {
    WorldTurnClaim {
        id: Uuid::new_v4(),
        user_id,
        novel_id,
        request_fingerprint: vec![7; 32],
        action: world_turn_action(),
        expected_turn_number,
        resolution: None,
    }
}

fn world_turn_action_to(location_id: &str, intent: &str) -> WorldAction {
    WorldAction {
        kind: WorldActionKind::Travel,
        target_id: Some(location_id.into()),
        intent: intent.into(),
    }
}

async fn world_turn_acquire(repo: &PgWorldTurnRepository, claim: &WorldTurnClaim) -> i64 {
    match repo.begin_turn(claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reservation: {result:?}"),
    }
}

#[tokio::test]
async fn world_turn_concurrent_same_key_acquires_exactly_once() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, _context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let claim = world_turn_claim(user_id, novel_id);
    // Real concurrent race on the same key: the loser's INSERT blocks on the
    // partial unique index and then observes the committed in_progress row.
    let (first, second) = tokio::join!(repo.begin_turn(&claim), repo.begin_turn(&claim));
    let results = [first.unwrap(), second.unwrap()];
    let acquired = results
        .iter()
        .filter(|result| matches!(result, BeginWorldTurn::Acquired { .. }))
        .count();
    let in_progress = results
        .iter()
        .filter(|result| matches!(result, BeginWorldTurn::InProgress { .. }))
        .count();
    assert_eq!(acquired, 1, "exactly one begin must acquire the key");
    assert_eq!(in_progress, 1, "the loser must observe InProgress");
    let status: String = sqlx::query_scalar("SELECT status FROM world_turns WHERE id = $1")
        .bind(claim.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(status, "in_progress");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM world_turns WHERE id = $1")
            .bind(claim.id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        1,
        "a key race must never insert two rows"
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_completed_key_replays_and_cannot_commit_twice() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let claim = world_turn_claim(user_id, novel_id);
    let transition = world_turn_transition();
    let attempt = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reservation: {result:?}"),
    };
    let completed = repo
        .complete_turn(&claim, attempt, &transition, &context)
        .await
        .unwrap();
    // Completed-key replay returns the stored result, entity-equal.
    let replayed = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Completed {
            result: replayed, ..
        } => *replayed,
        result => panic!("unexpected replay: {result:?}"),
    };
    assert_eq!(replayed, completed);
    // A second commit on the completed key is fenced — with the same attempt
    // and with a bumped attempt: no duplicate commit either way.
    assert!(repo
        .complete_turn(&claim, attempt, &transition, &context)
        .await
        .is_err());
    assert!(repo
        .complete_turn(&claim, attempt + 1, &transition, &context)
        .await
        .is_err());
    let (status, turn_number): (String, i64) = sqlx::query_as(
        "SELECT status, (state #>> '{open_world,turn_number}')::BIGINT FROM world_turns w \
         JOIN world_states s ON s.user_id = w.user_id AND s.novel_id = w.novel_id \
         WHERE w.id = $1",
    )
    .bind(claim.id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(status, "completed");
    assert_eq!(turn_number, 1, "the world state must advance exactly once");
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_pending_projection_blocks_only_new_keys_until_terminal() {
    for terminal in [
        MemoryProjectionStatus::Saved,
        MemoryProjectionStatus::Skipped,
    ] {
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .connect(&db_url())
            .await
            .unwrap();
        let (user_id, novel_id, context) = seed_world_turn(&pool).await;
        let repo = PgWorldTurnRepository::new(pool.clone());
        let original = world_turn_claim(user_id, novel_id);
        let attempt = world_turn_acquire(&repo, &original).await;
        let completed = repo
            .complete_turn(&original, attempt, &world_turn_transition(), &context)
            .await
            .unwrap();
        let successor = world_turn_claim_at(user_id, novel_id, 1);
        let stale_successor = world_turn_claim_at(user_id, novel_id, 0);

        match repo.begin_turn(&original).await.unwrap() {
            BeginWorldTurn::Completed {
                result,
                memory_projection,
            } => {
                assert_eq!(*result, completed);
                assert_eq!(memory_projection, MemoryProjectionStatus::Pending);
            }
            result => panic!("pending exact replay was not prioritized: {result:?}"),
        }
        assert!(matches!(
            repo.begin_turn(&successor).await.unwrap(),
            BeginWorldTurn::InProgress {
                retry_after_seconds: 1..
            }
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM world_turns WHERE id = $1")
                .bind(successor.id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0,
            "a blocked successor must not reserve a row"
        );
        assert!(matches!(
            repo.begin_turn(&stale_successor).await.unwrap(),
            BeginWorldTurn::Stale
        ));
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM world_turns WHERE id = $1")
                .bind(stale_successor.id)
                .fetch_one(&pool)
                .await
                .unwrap(),
            0,
            "an old-view successor must not reserve a row"
        );

        assert!(repo
            .finish_memory_projection(original.id, user_id, novel_id, terminal)
            .await
            .unwrap());
        assert!(matches!(
            repo.begin_turn(&stale_successor).await.unwrap(),
            BeginWorldTurn::Stale
        ));
        let successor_attempt = world_turn_acquire(&repo, &successor).await;
        assert!(repo
            .fail_turn(successor.id, successor_attempt, "test_cleanup")
            .await
            .unwrap());

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn world_turn_pending_projection_keeps_the_unique_slot_across_commit() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, _context) = seed_world_turn(&pool).await;
    let repo = Arc::new(PgWorldTurnRepository::new(pool.clone()));
    let original = world_turn_claim(user_id, novel_id);
    world_turn_acquire(repo.as_ref(), &original).await;

    // Hold the status transition open after PostgreSQL has changed the partial
    // unique-index entry but before commit. A competing INSERT must wait, then
    // still conflict with the completed+pending row after this transaction
    // commits; it must never slip into the released in-progress slot.
    let mut commit = pool.begin().await.unwrap();
    sqlx::query(
        "UPDATE world_turns \
         SET status = 'completed', lease_expires_at = NULL, \
             transition = '{}'::jsonb, result = '{}'::jsonb, \
             completed_at = NOW(), updated_at = NOW() \
         WHERE id = $1",
    )
    .bind(original.id)
    .execute(&mut *commit)
    .await
    .unwrap();

    let successor = world_turn_claim(user_id, novel_id);
    let successor_id = successor.id;
    let competing_repo = repo.clone();
    let mut competing =
        tokio::spawn(async move { competing_repo.begin_turn(&successor).await.unwrap() });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), &mut competing)
            .await
            .is_err(),
        "the competing reservation must wait for the committing slot owner"
    );

    commit.commit().await.unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), competing)
        .await
        .expect("competing reservation did not resume after commit")
        .unwrap();
    assert!(matches!(
        outcome,
        BeginWorldTurn::InProgress {
            retry_after_seconds: 1..
        }
    ));
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM world_turns WHERE id = $1")
            .bind(successor_id)
            .fetch_one(&pool)
            .await
            .unwrap(),
        0,
        "the waiting reservation must not create a second row"
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_persists_and_exactly_replays_the_server_action_check() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let resolution = ActionCheck {
        schema_version: 1,
        canon_model_version: 1,
        template_prompt_version: "novel-game-rules-v1".into(),
        attribute_key: "vigor".into(),
        attribute_label: "身法".into(),
        score: 12,
        modifier: 1,
        roll: 14,
        difficulty_class: 13,
        total: 15,
        succeeded: true,
    };
    let claim = WorldTurnClaim {
        resolution: Some(resolution.clone()),
        ..world_turn_claim(user_id, novel_id)
    };
    let attempt = world_turn_acquire(&repo, &claim).await;
    let completed = repo
        .complete_turn(&claim, attempt, &world_turn_transition(), &context)
        .await
        .unwrap();
    let (replayed, memory_projection) = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Completed {
            result,
            memory_projection,
        } => (*result, memory_projection),
        outcome => panic!("unexpected replay: {outcome:?}"),
    };
    let journal = repo.journal(user_id, novel_id, 10).await.unwrap();

    assert_eq!(completed.resolution, Some(resolution.clone()));
    assert_eq!(replayed, completed);
    assert_eq!(memory_projection, MemoryProjectionStatus::Pending);
    assert_eq!(journal[0].resolution, Some(resolution.clone()));
    assert_eq!(
        sqlx::query_scalar::<_, serde_json::Value>(
            "SELECT resolution FROM world_turns WHERE id = $1",
        )
        .bind(claim.id)
        .fetch_one(&pool)
        .await
        .unwrap(),
        serde_json::to_value(resolution).unwrap(),
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn legacy_world_turn_replays_and_remains_in_the_journal() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let claim = world_turn_claim(user_id, novel_id);
    let mut transition = world_turn_transition();
    transition.prompt_version = "world-turn-v1".into();
    let attempt = world_turn_acquire(&repo, &claim).await;

    let completed = repo
        .complete_turn(&claim, attempt, &transition, &context)
        .await
        .unwrap();
    let replayed = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Completed {
            result: replayed, ..
        } => *replayed,
        result => panic!("unexpected replay: {result:?}"),
    };
    let journal = repo.journal(user_id, novel_id, 10).await.unwrap();

    assert_eq!(replayed, completed);
    assert_eq!(journal.len(), 1);
    assert_eq!(journal[0].transition.prompt_version, "world-turn-v1");
    assert_eq!(
        journal[0].transition.rendered_narrative,
        transition.rendered_narrative
    );
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_renew_fences_stale_attempts() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, _context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let claim = world_turn_claim(user_id, novel_id);
    let attempt = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reservation: {result:?}"),
    };
    assert!(repo.renew_turn(claim.id, attempt).await.unwrap());
    // A valid renew must actually extend the lease deadline.
    let deadline: chrono::DateTime<chrono::Utc> =
        sqlx::query_scalar("SELECT lease_expires_at FROM world_turns WHERE id = $1")
            .bind(claim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        deadline > chrono::Utc::now() + chrono::Duration::minutes(1),
        "a valid renew must extend the lease"
    );
    assert!(
        !repo.renew_turn(claim.id, attempt + 1).await.unwrap(),
        "a stale attempt must not renew the lease"
    );
    assert!(repo.renew_turn(claim.id, attempt).await.unwrap());
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_expired_lease_is_reclaimed_or_superseded() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, _context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let claim = world_turn_claim(user_id, novel_id);
    let attempt = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reservation: {result:?}"),
    };
    // Force lease expiry instead of waiting out the 2-minute lease.
    sqlx::query(
        "UPDATE world_turns SET lease_expires_at = NOW() - INTERVAL '1 minute' WHERE id = $1",
    )
    .bind(claim.id)
    .execute(&pool)
    .await
    .unwrap();
    // Same-key reclaim bumps the attempt atomically.
    let reclaimed = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reclaim: {result:?}"),
    };
    assert_eq!(reclaimed, attempt + 1);
    // A fresh key while an expired active row exists supersedes it.
    sqlx::query(
        "UPDATE world_turns SET lease_expires_at = NOW() - INTERVAL '1 minute' WHERE id = $1",
    )
    .bind(claim.id)
    .execute(&pool)
    .await
    .unwrap();
    let fresh = WorldTurnClaim {
        id: Uuid::new_v4(),
        ..claim.clone()
    };
    assert!(matches!(
        repo.begin_turn(&fresh).await.unwrap(),
        BeginWorldTurn::Acquired { .. }
    ));
    let (status, failure_code): (String, Option<String>) =
        sqlx::query_as("SELECT status, failure_code FROM world_turns WHERE id = $1")
            .bind(claim.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "failed");
    assert_eq!(failure_code.as_deref(), Some("superseded"));
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_failed_key_reclaim_supersedes_expired_other_owner() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, _context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let failed = world_turn_claim(user_id, novel_id);
    let failed_attempt = world_turn_acquire(&repo, &failed).await;
    assert!(repo
        .fail_turn(failed.id, failed_attempt, "llm_error")
        .await
        .unwrap());

    let active = WorldTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![8; 32],
        ..failed.clone()
    };
    assert_eq!(world_turn_acquire(&repo, &active).await, 1);
    sqlx::query(
        "UPDATE world_turns SET lease_expires_at = NOW() - INTERVAL '1 minute' WHERE id = $1",
    )
    .bind(active.id)
    .execute(&pool)
    .await
    .unwrap();

    let recovered_attempt = match repo.begin_turn(&failed).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("failed key was not recovered: {result:?}"),
    };
    assert_eq!(recovered_attempt, failed_attempt + 1);
    let (active_status, active_failure): (String, Option<String>) =
        sqlx::query_as("SELECT status, failure_code FROM world_turns WHERE id = $1")
            .bind(active.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(active_status, "failed");
    assert_eq!(active_failure.as_deref(), Some("superseded"));

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_failed_key_reclaim_preserves_live_other_owner() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, _context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let failed = world_turn_claim(user_id, novel_id);
    let failed_attempt = world_turn_acquire(&repo, &failed).await;
    assert!(repo
        .fail_turn(failed.id, failed_attempt, "llm_error")
        .await
        .unwrap());

    let active = WorldTurnClaim {
        id: Uuid::new_v4(),
        request_fingerprint: vec![8; 32],
        ..failed.clone()
    };
    assert_eq!(world_turn_acquire(&repo, &active).await, 1);
    let before: Vec<String> = sqlx::query_scalar(
        "SELECT ROW(id, status, attempt, failure_code, lease_expires_at, updated_at)::text \
         FROM world_turns WHERE id = ANY($1) ORDER BY id",
    )
    .bind(vec![failed.id, active.id])
    .fetch_all(&pool)
    .await
    .unwrap();

    assert!(matches!(
        repo.begin_turn(&failed).await.unwrap(),
        BeginWorldTurn::InProgress {
            retry_after_seconds: 1..
        }
    ));
    let after: Vec<String> = sqlx::query_scalar(
        "SELECT ROW(id, status, attempt, failure_code, lease_expires_at, updated_at)::text \
         FROM world_turns WHERE id = ANY($1) ORDER BY id",
    )
    .bind(vec![failed.id, active.id])
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        after, before,
        "a live owner and the failed key must not change"
    );

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_replay_does_not_advance_state() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, context) = seed_world_turn(&pool).await;
    let repo = PgWorldTurnRepository::new(pool.clone());
    let claim = world_turn_claim(user_id, novel_id);
    let transition = world_turn_transition();
    let attempt = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reservation: {result:?}"),
    };
    let completed = repo
        .complete_turn(&claim, attempt, &transition, &context)
        .await
        .unwrap();
    let state_after = completed.world_state.state.clone();
    // Replay the completed key: zero writes, no state advance.
    let replayed = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Completed {
            result: replayed, ..
        } => *replayed,
        result => panic!("unexpected replay: {result:?}"),
    };
    assert_eq!(replayed.world_state.state, state_after);
    let (turn_number, attempt_now): (i64, i64) = sqlx::query_as(
        "SELECT (state #>> '{open_world,turn_number}')::BIGINT, \
         (SELECT attempt FROM world_turns WHERE id = $1) \
         FROM world_states WHERE user_id = $2 AND novel_id = $3",
    )
    .bind(claim.id)
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(turn_number, 1);
    assert_eq!(attempt_now, 1, "replay must not bump the attempt");
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_journal_rebuilds_equivalent_state() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, context) = seed_world_turn(&pool).await;
    let world_state_repo = PgWorldStateRepository::new(pool.clone());
    let pre_state = world_state_repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap();
    let repo = PgWorldTurnRepository::new(pool.clone());
    let claim = world_turn_claim(user_id, novel_id);
    let transition = world_turn_transition();
    let attempt = match repo.begin_turn(&claim).await.unwrap() {
        BeginWorldTurn::Acquired { attempt, .. } => attempt,
        result => panic!("unexpected reservation: {result:?}"),
    };
    let completed = repo
        .complete_turn(&claim, attempt, &transition, &context)
        .await
        .unwrap();
    // Rebuilding from the committed journal must reproduce equivalent
    // authoritative state (H4 exit evidence: checkpoint + journal replay).
    let journal = repo.journal(user_id, novel_id, 100).await.unwrap();
    assert_eq!(journal.len(), 1);
    let entry = &journal[0];
    assert_eq!(entry.turn_id, claim.id);
    assert_eq!(entry.action, claim.action);
    assert_eq!(entry.transition, transition);
    let mut rebuilt = pre_state;
    rebuilt
        .apply_world_turn(entry.turn_id, &entry.action, &entry.transition, &context)
        .unwrap();
    assert_eq!(rebuilt.state, completed.world_state.state);
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn world_turn_multi_turn_journal_rebuilds_equivalent_state() {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&db_url())
        .await
        .unwrap();
    let (user_id, novel_id, context) = seed_world_turn(&pool).await;
    let world_state_repo = PgWorldStateRepository::new(pool.clone());
    // The turn-0 checkpoint the journal must rebuild from.
    let checkpoint = world_state_repo
        .get_or_create(user_id, novel_id)
        .await
        .unwrap();
    let repo = PgWorldTurnRepository::new(pool.clone());

    // Turn 1: travel to the tower, witness the siege event, gain a map.
    let claim1 = world_turn_claim(user_id, novel_id);
    let attempt1 = world_turn_acquire(&repo, &claim1).await;
    let transition1 = WorldTurnTransition {
        schema_version: WORLD_TURN_SCHEMA_VERSION,
        prompt_version: WORLD_TURN_PROMPT_VERSION.into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: 1,
        rendered_narrative: "你赶到北塔,目睹围城开始。".into(),
        events: vec![TransitionEvent {
            summary: "玩家目睹围城".into(),
            actor_character_ids: vec![],
            location_id: Some("north-tower".into()),
        }],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
        player_location_id: Some("north-tower".into()),
        inventory_additions: vec!["隐秘地图".into()],
        inventory_removals: vec![],
        knowledge_discoveries: vec![],
        faction_changes: vec![],
        canonical_event_change: Some(CanonicalEventChange {
            event_id: "siege-event".into(),
            status: CanonicalEventStatus::Witnessed,
            reason: "玩家目睹围城".into(),
        }),
    };
    repo.complete_turn(&claim1, attempt1, &transition1, &context)
        .await
        .unwrap();
    assert!(repo
        .finish_memory_projection(
            claim1.id,
            user_id,
            novel_id,
            MemoryProjectionStatus::Skipped,
        )
        .await
        .unwrap());

    // Turn 2: travel to the harbor, learn its secret.
    let claim2 = WorldTurnClaim {
        id: Uuid::new_v4(),
        user_id,
        novel_id,
        request_fingerprint: vec![8; 32],
        action: world_turn_action_to("harbor", "前往港湾"),
        expected_turn_number: 1,
        resolution: None,
    };
    let attempt2 = world_turn_acquire(&repo, &claim2).await;
    let transition2 = WorldTurnTransition {
        schema_version: WORLD_TURN_SCHEMA_VERSION,
        prompt_version: WORLD_TURN_PROMPT_VERSION.into(),
        canon_model_version: 1,
        canonical_checkpoint_chapter: 1,
        rendered_narrative: "你在港湾发现走私船的暗号。".into(),
        events: vec![TransitionEvent {
            summary: "玩家发现暗号".into(),
            actor_character_ids: vec![],
            location_id: Some("harbor".into()),
        }],
        relationship_changes: vec![],
        location_changes: vec![],
        thread_changes: vec![],
        player_location_id: Some("harbor".into()),
        inventory_additions: vec![],
        inventory_removals: vec![],
        knowledge_discoveries: vec!["港湾有走私暗号".into()],
        faction_changes: vec![],
        canonical_event_change: None,
    };
    let completed = repo
        .complete_turn(&claim2, attempt2, &transition2, &context)
        .await
        .unwrap();

    // Rebuilding from the turn-0 checkpoint through the committed journal
    // must reproduce the final world state exactly (H4 exit evidence).
    let journal = repo.journal(user_id, novel_id, 100).await.unwrap();
    assert_eq!(journal.len(), 2);
    assert_eq!(journal[0].turn_number, 1);
    assert_eq!(journal[1].turn_number, 2);
    let mut rebuilt = checkpoint;
    for entry in &journal {
        rebuilt
            .apply_world_turn(entry.turn_id, &entry.action, &entry.transition, &context)
            .unwrap();
    }
    assert_eq!(rebuilt.state, completed.world_state.state);
    assert_eq!(rebuilt.open_world().unwrap().unwrap().turn_number, 2);
    let player = rebuilt.player_entity().unwrap().unwrap();
    assert_eq!(player.location_id, "harbor");
    assert!(player.inventory.contains(&"隐秘地图".to_string()));
    assert!(player
        .discovered_knowledge
        .contains(&"港湾有走私暗号".to_string()));
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}
