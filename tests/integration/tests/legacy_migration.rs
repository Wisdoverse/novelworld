use agent_service::domain::ports::ReadinessProbe;
use agent_service::infrastructure::persistence::PgReadinessProbe;
use narrative_service::{
    domain::ports::ReadinessProbe as NarrativeReadinessPort,
    infrastructure::persistence::PgReadinessProbe as NarrativePgReadinessProbe,
};
use novel_service::{
    domain::ports::ReadinessProbe as NovelReadinessPort,
    infrastructure::persistence::PgReadinessProbe as NovelPgReadinessProbe,
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;

const LEGACY_SCHEMA: &str = include_str!("../fixtures/legacy_runtime_contract.sql");
const FRESH_SCHEMA: &str = include_str!("../../../infra/postgres/init.sql");
const RUNTIME_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0001_runtime_contract.sql");
const PROGRESS_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0002_reading_progress_contract.sql");
const CHAT_TURN_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0003_chat_turn_contract.sql");
const NARRATIVE_CHOICE_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0004_narrative_choice_contract.sql");
const SEED_REMOVAL_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0005_remove_default_seed.sql");
const RUNTIME_LLM_CONFIG_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0006_runtime_llm_config.sql");
const CHAPTER_LORE_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0007_chapter_lore_search.sql");
const LLM_THINKING_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0008_llm_thinking_mode.sql");
const NARRATIVE_ANCHOR_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0009_narrative_inline_anchor.sql");
const PLAYER_TIMELINE_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0010_player_timeline_chapters.sql");
const CANON_STORY_MODEL_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0011_canon_story_models.sql");
const NARRATIVE_TRANSITION_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0012_narrative_transitions.sql");
const LIVING_WORLD_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0013_living_world_turns.sql");
const SOURCE_FILE_STORAGE_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0014_source_file_storage.sql");
const DURABLE_IMPORT_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0015_durable_import_jobs.sql");
const ERASURE_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0016_erasure_records.sql");
const CANON_CHECKPOINT_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0017_canon_extraction_checkpoints.sql");
const CANON_CHECKPOINT_EXPANSION_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0018_expand_canon_checkpoint_source.sql");
const SHARED_NOVEL_SHELVES_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0019_share_novels_across_user_shelves.sql");
const ADVANCED_GAME_RULES_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0020_advanced_game_rules.sql");
const WORLD_TURN_MEMORY_PROJECTION_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0021_world_turn_memory_projection.sql");

fn db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://test:test@localhost:25432/novelworld_test".into())
}

#[tokio::test]
async fn fresh_schema_matches_replayable_chat_turn_contract() {
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url())
        .await
        .unwrap();
    sqlx::query("DROP DATABASE IF EXISTS novelworld_fresh_contract WITH (FORCE)")
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("CREATE DATABASE novelworld_fresh_contract")
        .execute(&admin)
        .await
        .unwrap();

    let options = PgConnectOptions::from_str(&db_url())
        .unwrap()
        .database("novelworld_fresh_contract");
    let fresh = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();

    sqlx::raw_sql(FRESH_SCHEMA).execute(&fresh).await.unwrap();
    let incomplete_user = uuid::Uuid::new_v4();
    let incomplete_novel = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'test-hash')")
        .bind(incomplete_user)
        .bind(format!("incomplete-ready-{incomplete_user}@test.invalid"))
        .execute(&fresh)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, status) VALUES ($1, $2, 'Incomplete ready', 'ready')",
    )
    .bind(incomplete_novel)
    .bind(incomplete_user)
    .execute(&fresh)
    .await
    .unwrap();
    let mismatched_novel = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, world_summary, genre, total_chapters, status) \
         VALUES ($1, $2, 'Mismatched ready', 'world', 'fantasy', 2, 'ready')",
    )
    .bind(mismatched_novel)
    .bind(incomplete_user)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chapters (novel_id, chapter_number, content) \
         VALUES ($1, 1, 'Only one durable chapter')",
    )
    .bind(mismatched_novel)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query("INSERT INTO characters (novel_id, name) VALUES ($1, 'Legacy character')")
        .bind(mismatched_novel)
        .execute(&fresh)
        .await
        .unwrap();
    let failed_novel = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, status, parse_error) \
         VALUES ($1, $2, 'Failed with internal detail', 'error', \
                 'provider secret from the old runtime')",
    )
    .bind(failed_novel)
    .bind(incomplete_user)
    .execute(&fresh)
    .await
    .unwrap();
    // total_chapters agrees with the row count, but chapter 2 is missing: the
    // novel must not be backfilled as a terminal completed import.
    let gapped_novel = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, world_summary, genre, total_chapters, status) \
         VALUES ($1, $2, 'Gapped ready', 'world', 'fantasy', 2, 'ready')",
    )
    .bind(gapped_novel)
    .bind(incomplete_user)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chapters (novel_id, chapter_number, content) \
         VALUES ($1, 1, 'First durable chapter'), ($1, 3, 'Third durable chapter')",
    )
    .bind(gapped_novel)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query("INSERT INTO characters (novel_id, name) VALUES ($1, 'Gapped character')")
        .bind(gapped_novel)
        .execute(&fresh)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO canon_story_models \
             (novel_id, model_version, schema_version, prompt_version, content) \
         VALUES ($1, 1, 1, 'legacy-backfill-test-v1', '{}'::jsonb)",
    )
    .bind(gapped_novel)
    .execute(&fresh)
    .await
    .unwrap();
    let blank_novel = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, world_summary, genre, total_chapters, status) \
         VALUES ($1, $2, 'Blank ready', 'world', 'fantasy', 1, 'ready')",
    )
    .bind(blank_novel)
    .bind(incomplete_user)
    .execute(&fresh)
    .await
    .unwrap();
    // Tabs and newlines survive a default BTRIM(), which strips only spaces.
    sqlx::query("INSERT INTO chapters (novel_id, chapter_number, content) VALUES ($1, 1, $2)")
        .bind(blank_novel)
        .bind("\t\n \t")
        .execute(&fresh)
        .await
        .unwrap();
    // Non-ASCII whitespace survives a POSIX [:space:] test under LC_CTYPE=C,
    // while Rust str::trim() strips it: the backfill must agree with Rust
    // regardless of the database locale.
    let non_ascii_blank_novel = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, world_summary, genre, total_chapters, status) \
         VALUES ($1, $2, 'Non-ASCII blank ready', 'world', 'fantasy', 1, 'ready')",
    )
    .bind(non_ascii_blank_novel)
    .bind(incomplete_user)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query("INSERT INTO chapters (novel_id, chapter_number, content) VALUES ($1, 1, $2)")
        .bind(non_ascii_blank_novel)
        .bind("\u{00a0}\u{3000}")
        .execute(&fresh)
        .await
        .unwrap();
    // An import interrupted before enrichment still carries total_chapters = 0
    // with every chapter durably stored, and must stay resumable.
    let resumable_novel = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO novels (id, user_id, title) VALUES ($1, $2, 'Interrupted import')")
        .bind(resumable_novel)
        .bind(incomplete_user)
        .execute(&fresh)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO chapters (novel_id, chapter_number, content) \
         VALUES ($1, 1, 'First durable chapter'), ($1, 2, 'Second durable chapter')",
    )
    .bind(resumable_novel)
    .execute(&fresh)
    .await
    .unwrap();
    for _ in 0..2 {
        sqlx::raw_sql(CHAT_TURN_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(NARRATIVE_CHOICE_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(SEED_REMOVAL_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(RUNTIME_LLM_CONFIG_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(CHAPTER_LORE_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(LLM_THINKING_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(NARRATIVE_ANCHOR_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(PLAYER_TIMELINE_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(CANON_STORY_MODEL_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(NARRATIVE_TRANSITION_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(LIVING_WORLD_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(SOURCE_FILE_STORAGE_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(DURABLE_IMPORT_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(ERASURE_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(CANON_CHECKPOINT_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(CANON_CHECKPOINT_EXPANSION_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(SHARED_NOVEL_SHELVES_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(ADVANCED_GAME_RULES_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
        sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
            .execute(&fresh)
            .await
            .unwrap();
    }

    let repaired_incomplete: (String, Option<String>, String, String, Option<String>) =
        sqlx::query_as(
            "SELECT novel.status::text, novel.parse_error, job.stage, job.status, job.failure_code \
             FROM novels AS novel \
             JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
             WHERE novel.id = $1",
        )
        .bind(incomplete_novel)
        .fetch_one(&fresh)
        .await
        .unwrap();
    assert_eq!(
        repaired_incomplete,
        (
            "error".into(),
            Some("Import data is incomplete after upgrade; retry or re-upload the source".into()),
            "source".into(),
            "failed".into(),
            Some("legacy_incomplete".into()),
        )
    );
    assert_eq!(
        sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT stage, status, failure_code FROM novel_import_jobs WHERE novel_id = $1",
        )
        .bind(mismatched_novel)
        .fetch_one(&fresh)
        .await
        .unwrap(),
        (
            "chapters".into(),
            "failed".into(),
            Some("legacy_incomplete".into()),
        )
    );
    for invalid_novel in [gapped_novel, blank_novel, non_ascii_blank_novel] {
        assert_eq!(
            sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
                "SELECT job.stage, job.status, job.failure_code, novel.parse_error \
                 FROM novels AS novel \
                 JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
                 WHERE novel.id = $1",
            )
            .bind(invalid_novel)
            .fetch_one(&fresh)
            .await
            .unwrap(),
            (
                "source".into(),
                "failed".into(),
                Some("legacy_chapters_invalid".into()),
                Some("Imported chapters are unusable after upgrade; re-upload the source".into()),
            )
        );
    }
    assert_eq!(
        sqlx::query_as::<_, (String, String, Option<String>)>(
            "SELECT stage, status, failure_code FROM novel_import_jobs WHERE novel_id = $1",
        )
        .bind(resumable_novel)
        .fetch_one(&fresh)
        .await
        .unwrap(),
        (
            "chapters".into(),
            "failed".into(),
            Some("interrupted_upgrade".into()),
        )
    );
    assert_eq!(
        sqlx::query_as::<_, (String, Option<String>)>(
            "SELECT job.failure_code, novel.parse_error \
             FROM novels AS novel \
             JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
             WHERE novel.id = $1",
        )
        .bind(failed_novel)
        .fetch_one(&fresh)
        .await
        .unwrap(),
        (
            "legacy_error".into(),
            Some("Previous import failed; retry or re-upload the source".into()),
        )
    );

    // The first revision of this migration recorded ready novels with gapped
    // chapters as completed imports. Replay must downgrade the misclassified
    // job instead of leaving the novel falsely ready forever.
    let misclassified_novel = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO novels (id, user_id, title, world_summary, genre, total_chapters, status) \
         VALUES ($1, $2, 'Misclassified completed', 'world', 'fantasy', 2, 'ready')",
    )
    .bind(misclassified_novel)
    .bind(incomplete_user)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chapters (novel_id, chapter_number, content) \
         VALUES ($1, 1, 'First durable chapter'), ($1, 3, 'Third durable chapter')",
    )
    .bind(misclassified_novel)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query("INSERT INTO characters (novel_id, name) VALUES ($1, 'Misclassified character')")
        .bind(misclassified_novel)
        .execute(&fresh)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO canon_story_models \
             (novel_id, model_version, schema_version, prompt_version, content) \
         VALUES ($1, 1, 1, 'legacy-misclassified-test-v1', '{}'::jsonb)",
    )
    .bind(misclassified_novel)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO novel_import_jobs (novel_id, stage, status) \
         VALUES ($1, 'completed', 'completed')",
    )
    .bind(misclassified_novel)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::raw_sql(DURABLE_IMPORT_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_as::<_, (String, String, Option<String>, String, Option<String>)>(
            "SELECT job.stage, job.status, job.failure_code, \
                    novel.status::text, novel.parse_error \
             FROM novels AS novel \
             JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
             WHERE novel.id = $1",
        )
        .bind(misclassified_novel)
        .fetch_one(&fresh)
        .await
        .unwrap(),
        (
            "source".into(),
            "failed".into(),
            Some("legacy_chapters_invalid".into()),
            "error".into(),
            Some("Imported chapters are unusable after upgrade; re-upload the source".into()),
        )
    );
    let downgrade_stamps =
        sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT job.updated_at, novel.updated_at \
             FROM novels AS novel \
             JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
             WHERE novel.id = $1",
        )
        .bind(misclassified_novel)
        .fetch_one(&fresh)
        .await
        .unwrap();
    sqlx::raw_sql(DURABLE_IMPORT_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_as::<_, (chrono::DateTime<chrono::Utc>, chrono::DateTime<chrono::Utc>)>(
            "SELECT job.updated_at, novel.updated_at \
                 FROM novels AS novel \
                 JOIN novel_import_jobs AS job ON job.novel_id = novel.id \
                 WHERE novel.id = $1",
        )
        .bind(misclassified_novel)
        .fetch_one(&fresh)
        .await
        .unwrap(),
        downgrade_stamps
    );

    let contract_exists: bool = sqlx::query_scalar(
        "SELECT to_regclass('public.chat_turns') IS NOT NULL \
             AND to_regclass('public.world_turns') IS NOT NULL \
             AND EXISTS ( \
                 SELECT 1 FROM information_schema.columns \
                 WHERE table_schema = 'public' \
                   AND table_name = 'chat_messages' \
                   AND column_name = 'turn_id' \
                   AND is_nullable = 'YES' \
             )",
    )
    .fetch_one(&fresh)
    .await
    .unwrap();
    assert!(contract_exists);

    let readiness = PgReadinessProbe::new(fresh.clone());
    assert!(readiness.is_ready().await);

    let narrative_readiness = NarrativePgReadinessProbe::new(fresh.clone());
    assert!(narrative_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.world_turns \
         DROP CONSTRAINT world_turns_resolution_check, \
         ADD CONSTRAINT world_turns_resolution_check CHECK (resolution IS NULL)",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!narrative_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.world_turns \
         DROP CONSTRAINT world_turns_resolution_check, \
         ADD CONSTRAINT world_turns_resolution_check \
         CHECK (resolution IS NULL OR pg_catalog.jsonb_typeof(resolution) = 'object')",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(narrative_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.world_turns \
         DROP COLUMN memory_projection_completed_at, \
         DROP COLUMN memory_projection_status CASCADE",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!narrative_readiness.is_ready().await);
    sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    assert!(narrative_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.world_turns \
         DROP CONSTRAINT world_turns_memory_projection_state_check, \
         ADD CONSTRAINT world_turns_memory_projection_state_check \
         CHECK (memory_projection_completed_at IS NULL OR status = 'completed')",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!narrative_readiness.is_ready().await);
    let constraint_drift = sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap_err();
    assert!(
        constraint_drift
            .to_string()
            .contains("world turn memory projection state constraint has an unexpected definition"),
        "unexpected migration error: {constraint_drift:#}"
    );
    // Production executes this explicitly transactional file through a
    // short-lived psql process, whose failed connection closes and rolls back.
    // This test deliberately keeps one pooled connection, so clear the failed
    // transaction before repairing the injected drift.
    sqlx::query("ROLLBACK").execute(&fresh).await.unwrap();
    sqlx::query(
        "ALTER TABLE public.world_turns \
         DROP CONSTRAINT world_turns_memory_projection_state_check",
    )
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    assert!(narrative_readiness.is_ready().await);
    sqlx::query(
        "ALTER TABLE public.world_turns \
         ALTER COLUMN memory_projection_status SET DEFAULT 'saved'::character varying",
    )
    .execute(&fresh)
    .await
    .unwrap();
    let column_drift = sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap_err();
    assert!(
        column_drift
            .to_string()
            .contains("world turn memory projection columns have an unexpected definition"),
        "unexpected migration error: {column_drift:#}"
    );
    sqlx::query("ROLLBACK").execute(&fresh).await.unwrap();
    sqlx::query(
        "ALTER TABLE public.world_turns \
         ALTER COLUMN memory_projection_status SET DEFAULT 'pending'::character varying",
    )
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    assert!(narrative_readiness.is_ready().await);

    // A failed concurrent unique-index build leaves an exact-name, exact-SQL
    // catalog object that is not enforced. Neither migration replay nor
    // readiness may mistake those strings for an authoritative uniqueness
    // fence.
    sqlx::query(
        "INSERT INTO public.world_states (user_id, novel_id) VALUES ($1, $2) \
         ON CONFLICT (user_id, novel_id) DO NOTHING",
    )
    .bind(incomplete_user)
    .bind(incomplete_novel)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query("DROP INDEX public.idx_world_turns_one_in_progress")
        .execute(&fresh)
        .await
        .unwrap();
    let invalid_turn_a = uuid::Uuid::new_v4();
    let invalid_turn_b = uuid::Uuid::new_v4();
    for (turn_id, fingerprint, expected_turn_number) in [
        (invalid_turn_a, vec![91_u8; 32], 0_i64),
        (invalid_turn_b, vec![92_u8; 32], 1_i64),
    ] {
        sqlx::query(
            "INSERT INTO public.world_turns ( \
                 id, user_id, novel_id, request_fingerprint, action, \
                 expected_turn_number, status, lease_expires_at \
             ) VALUES ($1, $2, $3, $4, '{}'::jsonb, $5, 'in_progress', \
                       NOW() + INTERVAL '5 minutes')",
        )
        .bind(turn_id)
        .bind(incomplete_user)
        .bind(incomplete_novel)
        .bind(fingerprint)
        .bind(expected_turn_number)
        .execute(&fresh)
        .await
        .unwrap();
    }
    let concurrent_build_error = sqlx::query(
        "CREATE UNIQUE INDEX CONCURRENTLY idx_world_turns_one_in_progress \
         ON public.world_turns(user_id, novel_id) \
         WHERE status = 'in_progress' \
            OR (status = 'completed' AND memory_projection_status = 'pending')",
    )
    .execute(&fresh)
    .await
    .unwrap_err();
    assert_eq!(
        concurrent_build_error
            .as_database_error()
            .and_then(|error| error.code().map(|code| code.into_owned()))
            .as_deref(),
        Some("23505"),
        "unexpected concurrent index error: {concurrent_build_error:#}"
    );
    let invalid_index_flags: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT indisunique, indisvalid, indisready, indislive \
         FROM pg_catalog.pg_index \
         WHERE indexrelid = \
             'public.idx_world_turns_one_in_progress'::pg_catalog.regclass \
           AND indrelid = 'public.world_turns'::pg_catalog.regclass",
    )
    .fetch_one(&fresh)
    .await
    .unwrap();
    assert!(invalid_index_flags.0);
    assert!(!invalid_index_flags.1 || !invalid_index_flags.2 || !invalid_index_flags.3);
    assert!(!narrative_readiness.is_ready().await);

    sqlx::query("DELETE FROM public.world_turns WHERE id = $1")
        .bind(invalid_turn_b)
        .execute(&fresh)
        .await
        .unwrap();
    sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    let repaired_index_flags: (bool, bool, bool, bool) = sqlx::query_as(
        "SELECT indisunique, indisvalid, indisready, indislive \
         FROM pg_catalog.pg_index \
         WHERE indexrelid = \
             'public.idx_world_turns_one_in_progress'::pg_catalog.regclass \
           AND indrelid = 'public.world_turns'::pg_catalog.regclass",
    )
    .fetch_one(&fresh)
    .await
    .unwrap();
    assert_eq!(repaired_index_flags, (true, true, true, true));
    assert!(narrative_readiness.is_ready().await);
    let duplicate_after_repair = sqlx::query(
        "INSERT INTO public.world_turns ( \
             id, user_id, novel_id, request_fingerprint, action, \
             expected_turn_number, status, lease_expires_at \
         ) VALUES ($1, $2, $3, $4, '{}'::jsonb, 2, 'in_progress', \
                   NOW() + INTERVAL '5 minutes')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(incomplete_user)
    .bind(incomplete_novel)
    .bind(vec![93_u8; 32])
    .execute(&fresh)
    .await
    .unwrap_err();
    assert_eq!(
        duplicate_after_repair
            .as_database_error()
            .and_then(|error| error.code().map(|code| code.into_owned()))
            .as_deref(),
        Some("23505")
    );

    let novel_readiness = NovelPgReadinessProbe::new(fresh.clone());
    assert!(novel_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.user_choices DROP CONSTRAINT user_choices_user_node_key; \
         ALTER TABLE public.user_choices ADD CONSTRAINT user_choices_user_node_key \
             UNIQUE(node_id, user_id)",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!narrative_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.user_choices DROP CONSTRAINT user_choices_user_node_key; \
         ALTER TABLE public.user_choices ADD CONSTRAINT user_choices_user_node_key \
             UNIQUE(user_id, node_id)",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(narrative_readiness.is_ready().await);

    // Constraint text renders the parent relative to search_path: once a
    // same-named clone leads the path, the healthy key deparses as
    // 'public.narrative_nodes(...)' while a key onto the clone deparses
    // exactly like the expected text. Readiness has to compare catalog
    // identity in both directions.
    sqlx::raw_sql(
        "CREATE SCHEMA decoy; \
         CREATE TABLE decoy.narrative_nodes ( \
             id UUID NOT NULL, novel_id UUID NOT NULL, chapter_number INTEGER NOT NULL, \
             UNIQUE (id, novel_id, chapter_number)); \
         INSERT INTO decoy.narrative_nodes (id, novel_id, chapter_number) \
             SELECT node_id, novel_id, chapter_number FROM public.user_choices",
    )
    .execute(&fresh)
    .await
    .unwrap();
    let shadowed = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(
            PgConnectOptions::from_str(&db_url())
                .unwrap()
                .database("novelworld_fresh_contract")
                .options([("search_path", "decoy,public")]),
        )
        .await
        .unwrap();
    let shadowed_readiness = NarrativePgReadinessProbe::new(shadowed.clone());
    assert!(shadowed_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.user_choices \
         DROP CONSTRAINT user_choices_node_scope_fkey, \
         ADD CONSTRAINT user_choices_node_scope_fkey \
             FOREIGN KEY (node_id, novel_id, chapter_number) \
             REFERENCES decoy.narrative_nodes(id, novel_id, chapter_number) \
             ON DELETE CASCADE",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!shadowed_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.user_choices \
         DROP CONSTRAINT user_choices_node_scope_fkey, \
         ADD CONSTRAINT user_choices_node_scope_fkey \
             FOREIGN KEY (node_id, novel_id, chapter_number) \
             REFERENCES public.narrative_nodes(id, novel_id, chapter_number) \
             ON DELETE CASCADE",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(shadowed_readiness.is_ready().await);
    sqlx::raw_sql(
        "CREATE TABLE decoy.world_states ( \
             user_id UUID NOT NULL, novel_id UUID NOT NULL, UNIQUE (user_id, novel_id)); \
         INSERT INTO decoy.world_states (user_id, novel_id) \
             SELECT user_id, novel_id FROM public.world_turns; \
         ALTER TABLE public.world_turns \
         DROP CONSTRAINT world_turns_world_state_fkey, \
         ADD CONSTRAINT world_turns_world_state_fkey \
             FOREIGN KEY (user_id, novel_id) \
             REFERENCES decoy.world_states(user_id, novel_id) ON DELETE CASCADE",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!shadowed_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.world_turns \
         DROP CONSTRAINT world_turns_world_state_fkey, \
         ADD CONSTRAINT world_turns_world_state_fkey \
             FOREIGN KEY (user_id, novel_id) \
             REFERENCES public.world_states(user_id, novel_id) ON DELETE CASCADE",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(shadowed_readiness.is_ready().await);
    shadowed.close().await;
    sqlx::query("DROP SCHEMA decoy CASCADE")
        .execute(&fresh)
        .await
        .unwrap();
    assert!(narrative_readiness.is_ready().await);

    sqlx::raw_sql(
        "DROP INDEX public.idx_chat_turns_one_in_progress; \
         CREATE UNIQUE INDEX idx_chat_turns_one_in_progress \
             ON public.chat_turns(character_id, user_id, novel_id) \
             WHERE status = 'in_progress'",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!readiness.is_ready().await);
    sqlx::query("DROP INDEX public.idx_chat_turns_one_in_progress")
        .execute(&fresh)
        .await
        .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    assert!(readiness.is_ready().await);

    sqlx::raw_sql(
        "DROP INDEX public.idx_chat_messages_turn_role_unique; \
         CREATE UNIQUE INDEX idx_chat_messages_turn_role_unique \
             ON public.chat_messages(role, turn_id) \
             WHERE turn_id IS NOT NULL",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!readiness.is_ready().await);
    sqlx::query("DROP INDEX public.idx_chat_messages_turn_role_unique")
        .execute(&fresh)
        .await
        .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    assert!(readiness.is_ready().await);

    let seed_id = uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
    let seed_hash = "$2b$12$LQv3c1yqBWVHxkd0LHAkCOYz6TiGniMnCGkzBMqVbNxoQyJXkBxKi";
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role) VALUES ($1, 'admin@novelworld.dev', $2, 'admin')",
    )
    .bind(seed_id)
    .bind(seed_hash)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::raw_sql(SEED_REMOVAL_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();
    assert!(
        !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1)")
            .bind(seed_id)
            .fetch_one(&fresh)
            .await
            .unwrap()
    );

    let owned_novel = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role) VALUES ($1, 'admin@novelworld.dev', $2, 'admin')",
    )
    .bind(seed_id)
    .bind(seed_hash)
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query("INSERT INTO novels (id, user_id, title) VALUES ($1, $2, 'Seed data')")
        .bind(owned_novel)
        .bind(seed_id)
        .execute(&fresh)
        .await
        .unwrap();
    let error = sqlx::raw_sql(SEED_REMOVAL_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap_err();
    assert!(error
        .to_string()
        .contains("known default admin credential owns product data"));
    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(owned_novel)
        .execute(&fresh)
        .await
        .unwrap();
    sqlx::raw_sql(SEED_REMOVAL_MIGRATION)
        .execute(&fresh)
        .await
        .unwrap();

    sqlx::query(
        "ALTER TABLE public.novel_import_jobs \
         DROP CONSTRAINT novel_import_jobs_stage_check, \
         ADD CONSTRAINT novel_import_jobs_stage_check CHECK (TRUE)",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!novel_readiness.is_ready().await);
    sqlx::query(
        "ALTER TABLE public.novel_import_jobs \
         DROP CONSTRAINT novel_import_jobs_stage_check, \
         ADD CONSTRAINT novel_import_jobs_stage_check \
             CHECK (stage IN ('source', 'chapters', 'enriched', 'completed'))",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(novel_readiness.is_ready().await);

    sqlx::query(
        "ALTER TABLE public.novel_import_jobs \
         DROP CONSTRAINT novel_import_jobs_novel_id_fkey, \
         ADD CONSTRAINT novel_import_jobs_novel_id_fkey \
             FOREIGN KEY (novel_id) REFERENCES public.novels(id)",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!novel_readiness.is_ready().await);
    sqlx::query(
        "ALTER TABLE public.novel_import_jobs \
         DROP CONSTRAINT novel_import_jobs_novel_id_fkey",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!novel_readiness.is_ready().await);
    // A cascading foreign key on any other column must not stand in for the
    // one that protects novel_id.
    sqlx::query(
        "ALTER TABLE public.novel_import_jobs \
         ADD COLUMN decoy_novel_id UUID \
             REFERENCES public.novels(id) ON DELETE CASCADE",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!novel_readiness.is_ready().await);
    sqlx::query("ALTER TABLE public.novel_import_jobs DROP COLUMN decoy_novel_id")
        .execute(&fresh)
        .await
        .unwrap();
    // A validated cascading key onto a clone of novels deparses identically to
    // the real one, so readiness has to compare catalog identity instead.
    sqlx::raw_sql(
        "CREATE TABLE public.novels_clone (id UUID PRIMARY KEY); \
         INSERT INTO public.novels_clone (id) \
             SELECT novel_id FROM public.novel_import_jobs; \
         ALTER TABLE public.novel_import_jobs \
         ADD CONSTRAINT novel_import_jobs_novel_id_fkey \
             FOREIGN KEY (novel_id) REFERENCES public.novels_clone(id) \
             ON DELETE CASCADE",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!novel_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.novel_import_jobs \
         DROP CONSTRAINT novel_import_jobs_novel_id_fkey; \
         DROP TABLE public.novels_clone",
    )
    .execute(&fresh)
    .await
    .unwrap();
    sqlx::query(
        "ALTER TABLE public.novel_import_jobs \
         ADD CONSTRAINT novel_import_jobs_novel_id_fkey \
             FOREIGN KEY (novel_id) REFERENCES public.novels(id) ON DELETE CASCADE",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(novel_readiness.is_ready().await);

    sqlx::query(
        "ALTER TABLE public.novel_import_jobs \
         DROP CONSTRAINT novel_import_jobs_state_check, \
         ADD CONSTRAINT novel_import_jobs_state_check CHECK (TRUE)",
    )
    .execute(&fresh)
    .await
    .unwrap();
    assert!(!novel_readiness.is_ready().await);

    fresh.close().await;
    sqlx::query("DROP DATABASE novelworld_fresh_contract WITH (FORCE)")
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
async fn legacy_world_turn_prose_is_retained_while_old_turns_become_terminal() {
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url())
        .await
        .unwrap();
    sqlx::query("DROP DATABASE IF EXISTS novelworld_projection_contract WITH (FORCE)")
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("CREATE DATABASE novelworld_projection_contract")
        .execute(&admin)
        .await
        .unwrap();

    let options = PgConnectOptions::from_str(&db_url())
        .unwrap()
        .database("novelworld_projection_contract");
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql(FRESH_SCHEMA).execute(&pool).await.unwrap();
    sqlx::raw_sql(
        "ALTER TABLE public.world_turns \
         DROP COLUMN memory_projection_completed_at, \
         DROP COLUMN memory_projection_status CASCADE; \
         CREATE UNIQUE INDEX idx_world_turns_one_in_progress \
         ON public.world_turns(user_id, novel_id) \
         WHERE status = 'in_progress'",
    )
    .execute(&pool)
    .await
    .unwrap();

    let user_id = uuid::Uuid::new_v4();
    let novel_id = uuid::Uuid::new_v4();
    let character_id = uuid::Uuid::new_v4();
    let non_protagonist_id = uuid::Uuid::new_v4();
    let legacy_turn_id = uuid::Uuid::new_v4();
    let collision_turn_id = uuid::Uuid::new_v4();
    let malformed_turn_id = uuid::Uuid::new_v4();
    let non_protagonist_turn_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'hash')")
        .bind(user_id)
        .bind(format!("projection-{user_id}@test.invalid"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO novels (id, user_id, title) VALUES ($1, $2, 'Projection')")
        .bind(novel_id)
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO characters \
             (id, novel_id, name, role, first_appearance_chapter) \
         VALUES ($1, $2, 'Witness', 'protagonist', 1), \
                ($3, $2, 'Bystander', 'supporting', 1)",
    )
    .bind(character_id)
    .bind(novel_id)
    .bind(non_protagonist_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO world_states (user_id, novel_id) VALUES ($1, $2)")
        .bind(user_id)
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();

    let result = serde_json::json!({
        "world_state": {
            "state": {
                "open_world": {
                    "entry_context": {
                        "checkpoint_chapter": 1,
                        "unlocked_through_chapter": 2
                    }
                }
            }
        }
    });
    for (index, turn_id, rendered_narrative) in [
        (
            0_i64,
            legacy_turn_id,
            "\u{3000}\u{00a0}\r\nlegacy projected prose\r\n\u{00a0}\u{3000}",
        ),
        (1_i64, collision_turn_id, "different committed prose"),
        (
            2_i64,
            non_protagonist_turn_id,
            "non protagonist exact prose",
        ),
    ] {
        let transition = serde_json::json!({
            "canonical_checkpoint_chapter": 1,
            "rendered_narrative": rendered_narrative
        });
        sqlx::query(
            "INSERT INTO world_turns ( \
                 id, user_id, novel_id, request_fingerprint, action, \
                 expected_turn_number, status, transition, result, completed_at \
             ) VALUES ($1, $2, $3, $4, '{}'::jsonb, $5, 'completed', $6, $7, NOW())",
        )
        .bind(turn_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(vec![index as u8 + 1; 32])
        .bind(index)
        .bind(transition)
        .bind(result.clone())
        .execute(&pool)
        .await
        .unwrap();
    }
    let malformed_transition = serde_json::json!({
        "rendered_narrative": "malformed legacy prose"
    });
    sqlx::query(
        "INSERT INTO world_turns ( \
             id, user_id, novel_id, request_fingerprint, action, \
             expected_turn_number, status, transition, result, completed_at \
         ) VALUES ($1, $2, $3, $4, '{}'::jsonb, 3, 'completed', $5, '{}'::jsonb, NOW())",
    )
    .bind(malformed_turn_id)
    .bind(user_id)
    .bind(novel_id)
    .bind(vec![4_u8; 32])
    .bind(malformed_transition)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO character_memories ( \
             id, character_id, user_id, novel_id, layer, content, importance, chapter_number \
         ) VALUES \
             ($1, $2, $3, $4, 'permanent', 'legacy projected prose', 7, 1), \
             ($5, $2, $3, $4, 'permanent', 'hostile collision', 7, 1), \
             ($6, $2, $3, $4, 'permanent', 'malformed legacy prose', 7, 1), \
             ($7, $8, $3, $4, 'permanent', 'non protagonist exact prose', 7, 1)",
    )
    .bind(legacy_turn_id)
    .bind(character_id)
    .bind(user_id)
    .bind(novel_id)
    .bind(collision_turn_id)
    .bind(malformed_turn_id)
    .bind(non_protagonist_turn_id)
    .bind(non_protagonist_id)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();

    let rows: (i64, i32, i32, i32) = sqlx::query_as(
        "SELECT \
             (SELECT COUNT(*) FROM character_memories WHERE id = $1), \
             (SELECT chapter_number FROM character_memories WHERE id = $2), \
             (SELECT chapter_number FROM character_memories WHERE id = $3), \
             (SELECT chapter_number FROM character_memories WHERE id = $4)",
    )
    .bind(legacy_turn_id)
    .bind(collision_turn_id)
    .bind(malformed_turn_id)
    .bind(non_protagonist_turn_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    // Migration cannot reconstruct the exact historical protagonist/witness
    // boundary, so it is deliberately lossless. Agent prompt consumers own
    // the fail-closed quarantine of the former producer class.
    assert_eq!(rows, (1, 1, 1, 1));
    let skipped_turns: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM world_turns \
         WHERE id IN ($1, $2, $3, $4) \
           AND memory_projection_status = 'skipped' \
           AND memory_projection_completed_at IS NOT NULL",
    )
    .bind(legacy_turn_id)
    .bind(collision_turn_id)
    .bind(malformed_turn_id)
    .bind(non_protagonist_turn_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(skipped_turns, 4);

    let new_pending_turn_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO world_turns ( \
             id, user_id, novel_id, request_fingerprint, action, \
             expected_turn_number, status, transition, result, completed_at \
         ) VALUES ($1, $2, $3, $4, '{}'::jsonb, 4, 'completed', \
             '{}'::jsonb, '{}'::jsonb, NOW())",
    )
    .bind(new_pending_turn_id)
    .bind(user_id)
    .bind(novel_id)
    .bind(vec![5_u8; 32])
    .execute(&pool)
    .await
    .unwrap();
    sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    let replay_preserved_pending: bool = sqlx::query_scalar(
        "SELECT memory_projection_status = 'pending' \
             AND memory_projection_completed_at IS NULL \
         FROM world_turns WHERE id = $1",
    )
    .bind(new_pending_turn_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(replay_preserved_pending);

    pool.close().await;
    sqlx::query("DROP DATABASE novelworld_projection_contract WITH (FORCE)")
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
async fn legacy_schema_upgrade_is_lossless_and_replay_safe() {
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url())
        .await
        .unwrap();
    sqlx::query("DROP DATABASE IF EXISTS novelworld_legacy_contract WITH (FORCE)")
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("CREATE DATABASE novelworld_legacy_contract")
        .execute(&admin)
        .await
        .unwrap();

    let options = PgConnectOptions::from_str(&db_url())
        .unwrap()
        .database("novelworld_legacy_contract");
    let legacy = PgPoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();

    sqlx::raw_sql(LEGACY_SCHEMA).execute(&legacy).await.unwrap();
    for _ in 0..2 {
        sqlx::raw_sql(RUNTIME_MIGRATION)
            .execute(&legacy)
            .await
            .unwrap();
        sqlx::raw_sql(PROGRESS_MIGRATION)
            .execute(&legacy)
            .await
            .unwrap();
    }

    sqlx::raw_sql(
        "CREATE FUNCTION decoy.now() RETURNS TIMESTAMPTZ \
         LANGUAGE SQL IMMUTABLE \
         AS $$ SELECT '2001-01-01 00:00:00+00'::TIMESTAMPTZ $$",
    )
    .execute(&legacy)
    .await
    .unwrap();
    sqlx::raw_sql(
        "CREATE DOMAIN decoy.regclass AS oid; \
         CREATE DOMAIN decoy.regnamespace AS oid; \
         CREATE DOMAIN decoy.uuid AS pg_catalog.uuid; \
         CREATE DOMAIN decoy.text AS pg_catalog.text; \
         CREATE TABLE decoy.pg_constraint (ignored INTEGER); \
         CREATE TABLE decoy.pg_index (ignored INTEGER); \
         CREATE TABLE decoy.pg_class (ignored INTEGER); \
         CREATE TABLE decoy.pg_am (ignored INTEGER)",
    )
    .execute(&legacy)
    .await
    .unwrap();

    let mut psql_url = reqwest::Url::parse(&db_url()).unwrap();
    psql_url.set_path("/novelworld_legacy_contract");
    for migration in [
        "0003_chat_turn_contract.sql",
        "0004_narrative_choice_contract.sql",
        "0005_remove_default_seed.sql",
        "0006_runtime_llm_config.sql",
        "0007_chapter_lore_search.sql",
        "0008_llm_thinking_mode.sql",
        "0009_narrative_inline_anchor.sql",
        "0010_player_timeline_chapters.sql",
        "0011_canon_story_models.sql",
        "0012_narrative_transitions.sql",
        "0013_living_world_turns.sql",
        "0014_source_file_storage.sql",
        "0015_durable_import_jobs.sql",
        "0016_erasure_records.sql",
        "0017_canon_extraction_checkpoints.sql",
        "0018_expand_canon_checkpoint_source.sql",
        "0019_share_novels_across_user_shelves.sql",
        "0020_advanced_game_rules.sql",
        "0021_world_turn_memory_projection.sql",
    ] {
        let migration_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../infra/postgres/migrations")
            .join(migration);
        let psql = tokio::process::Command::new("psql")
            .arg("--set=ON_ERROR_STOP=1")
            .arg("--file")
            .arg(migration_path)
            .arg("--dbname")
            .arg(psql_url.as_str())
            .env("PGOPTIONS", "-c search_path=decoy,pg_catalog")
            .output()
            .await
            .expect("psql must be installed for the production migration contract test");
        assert!(
            psql.status.success(),
            "psql migration {migration} failed: {}",
            String::from_utf8_lossy(&psql.stderr)
        );
    }

    let mut non_default_path = legacy.acquire().await.unwrap();
    sqlx::query("SET search_path TO decoy, pg_catalog")
        .execute(&mut *non_default_path)
        .await
        .unwrap();
    for _ in 0..2 {
        sqlx::raw_sql(PROGRESS_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(CHAT_TURN_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(NARRATIVE_CHOICE_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(SEED_REMOVAL_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(RUNTIME_LLM_CONFIG_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(CHAPTER_LORE_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(LLM_THINKING_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(NARRATIVE_ANCHOR_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(PLAYER_TIMELINE_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(CANON_STORY_MODEL_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(NARRATIVE_TRANSITION_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(LIVING_WORLD_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(SOURCE_FILE_STORAGE_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(DURABLE_IMPORT_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(ERASURE_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(CANON_CHECKPOINT_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(CANON_CHECKPOINT_EXPANSION_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(SHARED_NOVEL_SHELVES_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(ADVANCED_GAME_RULES_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
        sqlx::raw_sql(WORLD_TURN_MEMORY_PROJECTION_MIGRATION)
            .execute(&mut *non_default_path)
            .await
            .unwrap();
    }
    sqlx::query("RESET search_path")
        .execute(&mut *non_default_path)
        .await
        .unwrap();
    drop(non_default_path);

    let legacy_memory: (uuid::Uuid, uuid::Uuid, String, i16, Option<i32>) = sqlx::query_as(
        "SELECT id, novel_id, content, importance, chapter_number \
         FROM public.character_memories WHERE content = 'legacy memory'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(
        legacy_memory,
        (
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000004").unwrap(),
            uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap(),
            "legacy memory".into(),
            5,
            None,
        )
    );

    let chapter_context: Option<i32> = sqlx::query_scalar(
        "SELECT chapter_context FROM public.chat_messages WHERE content = 'legacy chat'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(chapter_context, Some(7));

    let legacy_chapter_content: String = sqlx::query_scalar(
        "SELECT content FROM public.chapters \
         WHERE id = '00000000-0000-0000-0000-000000000012'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert!(legacy_chapter_content.is_empty());

    let chapter_content_nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'chapters' \
         AND column_name = 'content'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(chapter_content_nullable, "NO");

    let legacy_turn_id: Option<uuid::Uuid> = sqlx::query_scalar(
        "SELECT turn_id FROM public.chat_messages WHERE content = 'legacy chat'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert!(legacy_turn_id.is_none());

    let turn_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM public.chat_turns")
        .fetch_one(&legacy)
        .await
        .unwrap();
    assert_eq!(turn_count, 0);

    let legacy_column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'chat_messages' \
         AND column_name = 'chapter_num'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(legacy_column_count, 0);

    let target_constraint_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_constraint \
         WHERE (conname = 'character_memories_novel_id_fkey' \
                AND conrelid = 'public.character_memories'::regclass) \
            OR (conname = 'reading_progress_current_chapter_check' \
                AND conrelid = 'public.reading_progress'::regclass) \
            OR (conname = 'reading_progress_identity_fields_check' \
                AND conrelid = 'public.reading_progress'::regclass)",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(target_constraint_count, 3);

    let player_timeline_contract: bool = sqlx::query_scalar(
        "SELECT to_regclass('public.player_chapters') IS NOT NULL \
         AND to_regclass('public.idx_narrative_nodes_canonical_chapter') IS NOT NULL \
         AND to_regclass('public.idx_narrative_nodes_player_chapter') IS NOT NULL",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert!(player_timeline_contract);

    let progress: (i32, String, Option<String>, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT current_chapter, reader_identity_type::text, reader_identity, reader_character_id \
         FROM public.reading_progress \
         WHERE id = '00000000-0000-0000-0000-000000000008'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(
        progress,
        (
            7,
            "character".into(),
            Some("Legacy Future Character".into()),
            Some(uuid::Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap()),
        )
    );

    let repaired_hole: (i32, String, Option<String>, Option<uuid::Uuid>) = sqlx::query_as(
        "SELECT current_chapter, reader_identity_type::text, reader_identity, reader_character_id \
         FROM public.reading_progress \
         WHERE id = '00000000-0000-0000-0000-000000000011'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(
        repaired_hole,
        (1, "self".into(), Some("Reader".into()), None)
    );

    let character_name: String = sqlx::query_scalar("SELECT name FROM public.characters LIMIT 1")
        .fetch_one(&legacy)
        .await
        .unwrap();
    assert_eq!(character_name, "Legacy Future Character");

    let memory_nullable: String = sqlx::query_scalar(
        "SELECT is_nullable FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'character_memories' \
         AND column_name = 'novel_id'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(memory_nullable, "NO");

    let row_counts: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT \
            (SELECT COUNT(*) FROM public.character_memories), \
            (SELECT COUNT(*) FROM public.chat_messages), \
            (SELECT COUNT(*) FROM public.narrative_nodes), \
            (SELECT COUNT(*) FROM public.user_choices), \
            (SELECT COUNT(*) FROM public.world_states)",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(row_counts, (1, 1, 1, 1, 1));

    let import_jobs: Vec<(String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT stage, status, attempt, failure_code \
         FROM public.novel_import_jobs ORDER BY novel_id",
    )
    .fetch_all(&legacy)
    .await
    .unwrap();
    // Both fixture novels carry gapped chapter numbers with unrecoverable blank
    // content, so neither may be resumed from its persisted chapters.
    assert_eq!(
        import_jobs,
        vec![
            (
                "source".into(),
                "failed".into(),
                0,
                Some("legacy_chapters_invalid".into()),
            ),
            (
                "source".into(),
                "failed".into(),
                0,
                Some("legacy_chapters_invalid".into()),
            ),
        ]
    );
    let unusable_novels: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM public.novels \
         WHERE id IN ( \
             '00000000-0000-0000-0000-000000000002', \
             '00000000-0000-0000-0000-000000000010' \
         ) \
           AND status::text = 'error' \
           AND parse_error = \
               'Imported chapters are unusable after upgrade; re-upload the source'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(unusable_novels, 2);

    // Compose replays every migration on each deployment; the converted
    // failures must not be rewritten and reordered every time.
    let stamped: Vec<(uuid::Uuid, chrono::DateTime<chrono::Utc>)> =
        sqlx::query_as("SELECT id, updated_at FROM public.novels ORDER BY id")
            .fetch_all(&legacy)
            .await
            .unwrap();
    sqlx::raw_sql(DURABLE_IMPORT_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();
    assert_eq!(
        sqlx::query_as::<_, (uuid::Uuid, chrono::DateTime<chrono::Utc>)>(
            "SELECT id, updated_at FROM public.novels ORDER BY id",
        )
        .fetch_all(&legacy)
        .await
        .unwrap(),
        stamped
    );

    let chat_turn_contract_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pg_constraint \
         WHERE (conname = 'chat_turns_state_check' \
                AND conrelid = 'public.chat_turns'::regclass) \
            OR (conname = 'chat_turns_identity_fields_check' \
                AND conrelid = 'public.chat_turns'::regclass) \
            OR (conname = 'chat_turns_request_fingerprint_check' \
                AND conrelid = 'public.chat_turns'::regclass) \
            OR (conname = 'chat_messages_turn_id_fkey' \
                AND conrelid = 'public.chat_messages'::regclass)",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(chat_turn_contract_count, 4);

    let turn_index_is_valid: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM pg_index AS index_definition \
             JOIN pg_class AS index_relation \
               ON index_relation.oid = index_definition.indexrelid \
             WHERE index_relation.relnamespace = 'public'::regnamespace \
               AND index_relation.relname = 'idx_chat_messages_turn_role_unique' \
               AND index_definition.indrelid = 'public.chat_messages'::regclass \
               AND index_definition.indisunique \
               AND pg_get_indexdef(index_definition.indexrelid, 1, true) = 'turn_id' \
               AND pg_get_indexdef(index_definition.indexrelid, 2, true) = 'role' \
               AND pg_get_expr(index_definition.indpred, index_definition.indrelid) = \
                   '(turn_id IS NOT NULL)' \
         )",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert!(turn_index_is_valid);

    let one_in_progress_index_is_valid: bool = sqlx::query_scalar(
        "SELECT EXISTS ( \
             SELECT 1 FROM pg_index AS index_definition \
             JOIN pg_class AS index_relation \
               ON index_relation.oid = index_definition.indexrelid \
             WHERE index_relation.relnamespace = 'public'::regnamespace \
               AND index_relation.relname = 'idx_chat_turns_one_in_progress' \
               AND index_definition.indrelid = 'public.chat_turns'::regclass \
               AND index_definition.indisunique \
               AND pg_get_indexdef(index_definition.indexrelid, 1, true) = 'user_id' \
               AND pg_get_indexdef(index_definition.indexrelid, 2, true) = 'character_id' \
               AND pg_get_indexdef(index_definition.indexrelid, 3, true) = 'novel_id' \
               AND pg_get_expr(index_definition.indpred, index_definition.indrelid) = \
                   '((status)::text = ''in_progress''::text)' \
         )",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert!(one_in_progress_index_is_valid);

    sqlx::query("ALTER TABLE public.chat_turns ALTER COLUMN attempt DROP NOT NULL")
        .execute(&legacy)
        .await
        .unwrap();
    let turn_column_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(turn_column_error
        .to_string()
        .contains("chat turns columns have an unexpected definition"));
    sqlx::query("ALTER TABLE public.chat_turns ALTER COLUMN attempt SET NOT NULL")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query(
        "ALTER TABLE public.chat_messages \
         ALTER COLUMN turn_id SET DEFAULT \
             '00000000-0000-0000-0000-000000000099'::uuid",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let message_turn_column_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(message_turn_column_error
        .to_string()
        .contains("chat messages turn column has an unexpected definition"));
    sqlx::query("ALTER TABLE public.chat_messages ALTER COLUMN turn_id DROP DEFAULT")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::raw_sql(
        "ALTER TABLE public.chat_messages \
         DROP CONSTRAINT chat_messages_turn_id_fkey; \
         ALTER TABLE public.chat_turns \
         DROP CONSTRAINT chat_turns_pkey, \
         ADD CONSTRAINT chat_turns_pkey PRIMARY KEY (user_id)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let turn_primary_key_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(turn_primary_key_error
        .to_string()
        .contains("chat turns primary key has an unexpected definition"));
    sqlx::query("ALTER TABLE public.chat_turns DROP CONSTRAINT chat_turns_pkey")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query(
        "ALTER TABLE public.chat_turns \
         DROP CONSTRAINT chat_turns_state_check, \
         ADD CONSTRAINT chat_turns_state_check CHECK (TRUE)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let state_constraint_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(state_constraint_error
        .to_string()
        .contains("chat turns state constraint has an unexpected definition"));
    sqlx::query("ALTER TABLE public.chat_turns DROP CONSTRAINT chat_turns_state_check")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query(
        "ALTER TABLE public.chat_turns \
         DROP CONSTRAINT chat_turns_request_fingerprint_check, \
         ADD CONSTRAINT chat_turns_request_fingerprint_check \
             CHECK (octet_length(request_fingerprint) > 0)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let fingerprint_constraint_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(fingerprint_constraint_error
        .to_string()
        .contains("request fingerprint constraint has an unexpected definition"));
    sqlx::query(
        "ALTER TABLE public.chat_turns \
         DROP CONSTRAINT chat_turns_request_fingerprint_check",
    )
    .execute(&legacy)
    .await
    .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query(
        "ALTER TABLE public.chat_messages \
         DROP CONSTRAINT chat_messages_turn_id_fkey, \
         ADD CONSTRAINT chat_messages_turn_id_fkey \
             FOREIGN KEY (turn_id) REFERENCES public.chat_turns(id)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let turn_foreign_key_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(turn_foreign_key_error
        .to_string()
        .contains("chat messages turn foreign key has an unexpected definition"));
    sqlx::query(
        "ALTER TABLE public.chat_messages \
         DROP CONSTRAINT chat_messages_turn_id_fkey",
    )
    .execute(&legacy)
    .await
    .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query("DROP INDEX public.idx_chat_messages_turn_role_unique")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::query(
        "CREATE UNIQUE INDEX idx_chat_messages_turn_role_unique \
         ON public.chat_messages(role, turn_id) WHERE turn_id IS NOT NULL",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let turn_index_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(turn_index_error
        .to_string()
        .contains("chat messages turn/role index has an unexpected definition"));
    sqlx::query("DROP INDEX public.idx_chat_messages_turn_role_unique")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query("DROP INDEX public.idx_chat_turns_one_in_progress")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::query(
        "CREATE UNIQUE INDEX idx_chat_turns_one_in_progress \
         ON public.chat_turns(character_id, user_id, novel_id) \
         WHERE status = 'in_progress'",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let one_in_progress_index_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(one_in_progress_index_error
        .to_string()
        .contains("chat turns one-in-progress index has an unexpected definition"));
    sqlx::query("DROP INDEX public.idx_chat_turns_one_in_progress")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query("DROP INDEX public.idx_chat_turns_one_in_progress")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::raw_sql(
        "INSERT INTO public.chat_turns ( \
             id, user_id, character_id, novel_id, request_fingerprint, \
             chapter_context, reader_identity_type, deviation_mode, status, \
             lease_expires_at \
         ) VALUES \
             ( \
                 '00000000-0000-0000-0000-000000000020', \
                 '00000000-0000-0000-0000-000000000001', \
                 '00000000-0000-0000-0000-000000000003', \
                 '00000000-0000-0000-0000-000000000002', \
                 decode(repeat('01', 32), 'hex'), 7, 'self', 'canon', \
                 'in_progress', NOW() + INTERVAL '5 minutes' \
             ), \
             ( \
                 '00000000-0000-0000-0000-000000000021', \
                 '00000000-0000-0000-0000-000000000001', \
                 '00000000-0000-0000-0000-000000000003', \
                 '00000000-0000-0000-0000-000000000002', \
                 decode(repeat('02', 32), 'hex'), 7, 'self', 'canon', \
                 'in_progress', NOW() + INTERVAL '5 minutes' \
             )",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let duplicate_active_turn_error = sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(duplicate_active_turn_error
        .to_string()
        .contains("cannot enforce one in-progress chat turn"));
    sqlx::query(
        "DELETE FROM public.chat_turns \
         WHERE id IN ( \
             '00000000-0000-0000-0000-000000000020', \
             '00000000-0000-0000-0000-000000000021' \
         )",
    )
    .execute(&legacy)
    .await
    .unwrap();
    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query(
        "ALTER TABLE public.reading_progress \
         DROP CONSTRAINT reading_progress_current_chapter_check, \
         ADD CONSTRAINT reading_progress_current_chapter_check CHECK (current_chapter >= 0)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let chapter_constraint_error = sqlx::raw_sql(PROGRESS_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(chapter_constraint_error
        .to_string()
        .contains("chapter constraint has an unexpected definition"));
    sqlx::query(
        "ALTER TABLE public.reading_progress \
         DROP CONSTRAINT reading_progress_current_chapter_check",
    )
    .execute(&legacy)
    .await
    .unwrap();
    sqlx::raw_sql(PROGRESS_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::raw_sql(
        "INSERT INTO public.users (id, email, password_hash) VALUES \
             ('00000000-0000-0000-0000-000000000016', \
              'legacy-repair@test.invalid', 'legacy-test-hash'); \
         INSERT INTO public.novels (id, user_id, title, total_chapters) VALUES \
             ('00000000-0000-0000-0000-000000000017', \
              '00000000-0000-0000-0000-000000000016', 'Pending repair', 3); \
         INSERT INTO public.characters (id, novel_id, name, first_appearance_chapter) VALUES \
             ('00000000-0000-0000-0000-000000000018', \
              '00000000-0000-0000-0000-000000000017', E'\\tPending repair\\t', 1); \
         INSERT INTO public.reading_progress \
             (id, user_id, novel_id, current_chapter, reader_identity_type) VALUES \
             ('00000000-0000-0000-0000-000000000019', \
              '00000000-0000-0000-0000-000000000016', \
              '00000000-0000-0000-0000-000000000017', 1, 'self')",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let no_chapter_error = sqlx::raw_sql(PROGRESS_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(no_chapter_error
        .to_string()
        .contains("novel has no readable chapter"));
    let unchanged_name: String = sqlx::query_scalar(
        "SELECT name FROM public.characters \
         WHERE id = '00000000-0000-0000-0000-000000000018'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(unchanged_name, "\tPending repair\t");

    sqlx::query(
        "INSERT INTO public.chapters (id, novel_id, chapter_number, content) VALUES \
         ('00000000-0000-0000-0000-000000000020', \
          '00000000-0000-0000-0000-000000000017', 1, '')",
    )
    .execute(&legacy)
    .await
    .unwrap();
    for _ in 0..2 {
        sqlx::raw_sql(PROGRESS_MIGRATION)
            .execute(&legacy)
            .await
            .unwrap();
    }
    let repaired_name: String = sqlx::query_scalar(
        "SELECT name FROM public.characters \
         WHERE id = '00000000-0000-0000-0000-000000000018'",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(repaired_name, "Pending repair");

    sqlx::query(
        "ALTER TABLE public.reading_progress \
         DROP CONSTRAINT reading_progress_identity_fields_check, \
         ADD CONSTRAINT reading_progress_identity_fields_check CHECK (TRUE)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let identity_constraint_error = sqlx::raw_sql(PROGRESS_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(identity_constraint_error
        .to_string()
        .contains("identity constraint has an unexpected definition"));
    sqlx::query(
        "ALTER TABLE public.reading_progress \
         DROP CONSTRAINT reading_progress_identity_fields_check",
    )
    .execute(&legacy)
    .await
    .unwrap();
    sqlx::raw_sql(PROGRESS_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap();

    sqlx::query("DROP INDEX public.idx_narrative_nodes_canonical_chapter")
        .execute(&legacy)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO public.narrative_nodes (id, novel_id, chapter_number) \
         VALUES ('00000000-0000-0000-0000-000000000007', \
                 '00000000-0000-0000-0000-000000000002', 7)",
    )
    .execute(&legacy)
    .await
    .unwrap();
    let duplicate_error = sqlx::raw_sql(RUNTIME_MIGRATION)
        .execute(&legacy)
        .await
        .unwrap_err();
    assert!(duplicate_error
        .to_string()
        .contains("duplicate novel/chapter rows exist"));

    // Account deletion is the authority on the upgraded deletion graph: legacy
    // foreign keys were created without ON DELETE CASCADE.
    sqlx::query("DELETE FROM public.users")
        .execute(&legacy)
        .await
        .unwrap();
    let remaining: (i64, i64, i64, i64, i64, i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM public.users), \
                (SELECT COUNT(*) FROM public.novels), \
                (SELECT COUNT(*) FROM public.chapters), \
                (SELECT COUNT(*) FROM public.characters), \
                (SELECT COUNT(*) FROM public.novel_import_jobs), \
                (SELECT COUNT(*) FROM public.reading_progress), \
                (SELECT COUNT(*) FROM public.chat_messages), \
                (SELECT COUNT(*) FROM public.character_memories), \
                (SELECT COUNT(*) FROM public.user_choices), \
                (SELECT COUNT(*) FROM public.world_states)",
    )
    .fetch_one(&legacy)
    .await
    .unwrap();
    assert_eq!(remaining, (0, 0, 0, 0, 0, 0, 0, 0, 0, 0));

    legacy.close().await;
    sqlx::query("DROP DATABASE novelworld_legacy_contract WITH (FORCE)")
        .execute(&admin)
        .await
        .unwrap();
}
