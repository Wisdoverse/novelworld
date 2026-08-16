use agent_service::domain::{
    entities::memory::{ChatMessage, Memory, MemoryLayer},
    repositories::{BeginChatTurn, ChatRepository, ChatTurnClaim, MemoryRepository},
};
use agent_service::infrastructure::persistence::{
    pg_chat_repo::PgChatRepository, pg_memory_repo::PgMemoryRepository,
};
use narrative_service::domain::{
    entities::{
        narrative_node::{NarrativeChoice, NarrativeNode},
        player_entity::PlayerEntity,
        world_session::{
            CharacterGoalRef, FactionStandingChange, ScheduledCanonEvent, WorldAction,
            WorldActionKind, WorldCharacterRef, WorldEntityRef, WorldEntryContext, WorldRuleRef,
            WorldTurnTransition, WORLD_TURN_PROMPT_VERSION, WORLD_TURN_SCHEMA_VERSION,
        },
    },
    repositories::{
        BeginWorldTurn, ChoiceCommit, NarrativeNodeRepository, UserChoiceRepository,
        WorldStateRepository, WorldTurnClaim, WorldTurnRepository,
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
    entities::novel::Novel,
    ports::{ImagePort, LlmPort, NovelLlmTask, PrivacyCleanupPort, SourceFileStorage},
    repositories::{
        CanonStoryModelRepository, ChapterRepository, CharacterRelationshipRecord,
        CharacterRepository, NovelRepository, ReadingProgressRepository,
        SourceFileDeletionRepository,
    },
    value_objects::{CharacterRole, ImportStage},
};
use novel_service::infrastructure::document::EbookTextExtractor;
use novel_service::infrastructure::persistence::{
    canon_story_model_pg_repo::PgCanonStoryModelRepository, chapter_pg_repo::ChapterPgRepository,
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
    assert!(!sqlx::query_scalar::<_, bool>(
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
        candidates.iter().any(|candidate| candidate.novel_id == novel.id),
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
    assert!(
        repo.fail_import(novel.id, claim.attempt, "seeded_failure", "seeded")
            .await
            .unwrap()
    );
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
async fn source_file_cleanup_intent_is_atomic_and_survives_account_cascade() {
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
    assert!(repository.insert_import(&model, 2).await.unwrap());
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
    let memories = memory_repo
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
    assert_eq!(memories.len(), 1);
    assert_eq!(memories[0].novel_id, novel_id);
    assert_eq!(
        memory_repo
            .search_similar(character_id, user_id, novel_id, &embedding, 1, 5)
            .await
            .unwrap()
            .len(),
        1
    );

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
        .find_recent(character_id, user_id, novel_id, 1, 10)
        .await
        .unwrap();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].reader_identity.as_deref(), Some("Reader"));
    assert_eq!(messages[1].chapter_context, Some(1));
    assert_eq!(
        chat_repo
            .find_by_character_user(character_id, user_id, novel_id, 1, 10, 0)
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
        1,
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
    assert_eq!(
        world_state_repo
            .create_player_entity(&competing)
            .await
            .unwrap()
            .id,
        player.id
    );

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
            transition: transition(
                chapter_number,
                format!("第{chapter_number}章的选择已经生效。"),
            ),
            rewritten_chapter_content: format!("Rewritten chapter {chapter_number}"),
        });
    }
    let (second, third) = tokio::join!(
        choice_repo.commit_choice(&more_drafts[0]),
        choice_repo.commit_choice(&more_drafts[1])
    );
    second.unwrap();
    third.unwrap();
    let valid_state: serde_json::Value =
        sqlx::query_scalar("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(valid_state["choices"].as_array().unwrap().len(), 3);

    sqlx::query(
        "UPDATE world_states SET state = jsonb_set(state, '{player_entity,unknown}', 'true'::jsonb) WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    assert!(choice_repo.commit_choice(&more_drafts[2]).await.is_err());
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
    assert!(choice_repo.commit_choice(&more_drafts[2]).await.is_err());
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
    choice_repo.commit_choice(&more_drafts[2]).await.unwrap();

    let world_context = WorldEntryContext {
        model_version: 1,
        checkpoint_chapter: 1,
        unlocked_through_chapter: 2,
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
            source_chapters: vec![2],
        }],
        character_goals: vec![CharacterGoalRef {
            id: "goal-1".into(),
            character_id,
            description: "Protect the tower.".into(),
            source_chapters: vec![1],
        }],
    };
    let started = world_state_repo
        .start_open_world(user_id, novel_id, &world_context)
        .await
        .unwrap();
    assert_eq!(started.open_world().unwrap().unwrap().turn_number, 0);
    let mut drifted_context = world_context.clone();
    drifted_context.model_version = 2;
    let resumed = world_state_repo
        .start_open_world(user_id, novel_id, &drifted_context)
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
        canonical_checkpoint_chapter: 1,
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
        BeginWorldTurn::Completed(replayed) => assert_eq!(*replayed, completed),
        result => panic!("completed world turn did not replay: {result:?}"),
    }
    let advanced_replay = WorldTurnClaim {
        expected_turn_number: 1,
        ..claim.clone()
    };
    match world_turn_repo.begin_turn(&advanced_replay).await.unwrap() {
        BeginWorldTurn::Completed(replayed) => assert_eq!(*replayed, completed),
        result => panic!("advanced completed world turn did not replay: {result:?}"),
    }
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
    assert_eq!(
        world_turn_repo
            .journal(user_id, novel_id, 100)
            .await
            .unwrap()
            .len(),
        1
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
