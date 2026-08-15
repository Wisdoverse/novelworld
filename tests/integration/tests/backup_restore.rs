//! Erasure-record and replay contracts behind `backup-restore-v1`
//! (SPEC 12.4.1, 12.4.2, 12.4.5 and `docs/BACKUP_RESTORE.md`).
//!
//! The scripted backup, the restore gate, and the drills live in
//! `infra/backup/` and `tests/e2e/backup_restore_drill.sh`; this file covers the
//! database contracts they depend on, including the retained-source re-queue
//! that the S3-less end-to-end topology cannot exercise through services.

use futures::StreamExt;
use narrative_service::{
    domain::ports::ReadinessProbe as NarrativeReadinessPort,
    infrastructure::persistence::PgReadinessProbe as NarrativePgReadinessProbe,
};
use novel_service::{
    domain::ports::{AccountExportPort, ReadinessProbe},
    infrastructure::persistence::{account_export::PgAccountExport, PgReadinessProbe},
};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgPool, Row};
use std::str::FromStr;
use uuid::Uuid;

const FRESH_SCHEMA: &str = include_str!("../../../infra/postgres/init.sql");
const CHAT_TURN_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0003_chat_turn_contract.sql");
const ERASURE_MIGRATION: &str =
    include_str!("../../../infra/postgres/migrations/0016_erasure_records.sql");

fn db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://test:test@localhost:25432/novelworld_test".into())
}

/// A scratch database carrying the production schema, so replay statements can
/// run against a whole installation without disturbing the shared test data.
async fn scratch_database(name: &str) -> PgPool {
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url())
        .await
        .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE IF EXISTS {name} WITH (FORCE)"
    )))
    .execute(&admin)
    .await
    .unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {name}")))
        .execute(&admin)
        .await
        .unwrap();
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect_with(
            PgConnectOptions::from_str(&db_url())
                .unwrap()
                .database(name),
        )
        .await
        .unwrap();
    sqlx::raw_sql(FRESH_SCHEMA).execute(&pool).await.unwrap();
    pool
}

async fn seed_account(pool: &PgPool, user_id: Uuid, novels: &[(Uuid, Option<String>)]) {
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'hash')")
        .bind(user_id)
        .bind(format!("erasure-{user_id}@test.invalid"))
        .execute(pool)
        .await
        .unwrap();
    for (novel_id, source_key) in novels {
        sqlx::query(
            "INSERT INTO novels (id, user_id, title, original_file_key, status) \
             VALUES ($1, $2, 'Erasure subject', $3, 'ready')",
        )
        .bind(novel_id)
        .bind(user_id)
        .bind(source_key)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO chapters (novel_id, chapter_number, content) \
             VALUES ($1, 1, 'First durable chapter'), ($1, 2, 'Second durable chapter')",
        )
        .bind(novel_id)
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn records(pool: &PgPool) -> Vec<(String, Uuid, Uuid, bool, bool)> {
    sqlx::query_as::<_, (String, Uuid, Uuid, bool, bool)>(
        "SELECT subject_type, subject_id, user_id, had_source, \
                source_requeued_at IS NOT NULL \
         FROM erasure_records ORDER BY subject_type, subject_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn outbox(pool: &PgPool) -> Vec<String> {
    sqlx::query_scalar::<_, String>("SELECT object_key FROM source_file_deletions ORDER BY 1")
        .fetch_all(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn every_deletion_path_writes_a_minimal_erasure_record() {
    let pool = scratch_database("novelworld_erasure_paths").await;
    let owner = Uuid::new_v4();
    let direct_novel = Uuid::new_v4();
    let cascaded_novel = Uuid::new_v4();
    let survivor = Uuid::new_v4();
    let survivor_novel = Uuid::new_v4();
    seed_account(
        &pool,
        owner,
        &[
            (direct_novel, None),
            (
                cascaded_novel,
                Some(format!("source-files/{owner}/{cascaded_novel}")),
            ),
        ],
    )
    .await;
    seed_account(&pool, survivor, &[(survivor_novel, None)]).await;

    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(direct_novel)
        .execute(&pool)
        .await
        .unwrap();
    // The account cascade must produce a per-novel record as well as the
    // account record, and must not remove the record it just wrote.
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();

    let mut expected = vec![
        // The cascaded novel held a retained source, the directly deleted one
        // did not: only the delete itself can see that, so the record carries it.
        ("novel".to_string(), cascaded_novel, owner, true, false),
        ("novel".to_string(), direct_novel, owner, false, false),
        ("user".to_string(), owner, owner, false, false),
    ];
    expected.sort();
    let mut actual = records(&pool).await;
    actual.sort();
    assert_eq!(actual, expected);

    // Minimal fields: the identifying payload is the subject type and UUIDs;
    // erased_at, had_source and source_requeued_at are operational bookkeeping
    // for replay. Nothing derived from content, profile, or credentials
    // (SPEC 12.4.1, 12.4.5).
    let columns: Vec<String> = sqlx::query_scalar(
        "SELECT column_name::text FROM information_schema.columns \
         WHERE table_schema = 'public' AND table_name = 'erasure_records' \
         ORDER BY ordinal_position",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        columns,
        vec![
            "subject_type",
            "subject_id",
            "user_id",
            "erased_at",
            "had_source",
            "source_requeued_at"
        ]
    );

    // Deleting the same subject again keeps the original deletion fact: replay
    // re-fires these triggers and must never move erased_at forward.
    let first_erasure: chrono::DateTime<chrono::Utc> = sqlx::query_scalar(
        "SELECT erased_at FROM erasure_records WHERE subject_type = 'novel' AND subject_id = $1",
    )
    .bind(direct_novel)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO novels (id, user_id, title) VALUES ($1, $2, 'Resurrected')")
        .bind(direct_novel)
        .bind(survivor)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(direct_novel)
        .execute(&pool)
        .await
        .unwrap();
    let (kept_user, kept_erasure): (Uuid, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "SELECT user_id, erased_at FROM erasure_records \
         WHERE subject_type = 'novel' AND subject_id = $1",
    )
    .bind(direct_novel)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(kept_erasure, first_erasure);
    assert_eq!(kept_user, owner);
}

#[tokio::test]
async fn erasure_replay_is_idempotent_and_requeues_each_source_key_once() {
    let pool = scratch_database("novelworld_erasure_replay").await;
    let owner = Uuid::new_v4();
    let keyed_novel = Uuid::new_v4();
    let unknown_novel = Uuid::new_v4();
    let keyless_novel = Uuid::new_v4();
    let keyed_source = format!("source-files/{owner}/{keyed_novel}");
    let reconstructed_source = format!("source-files/{owner}/{unknown_novel}");
    seed_account(&pool, owner, &[(keyed_novel, Some(keyed_source.clone()))]).await;

    // A record whose subject row is in no dump, carrying the had_source fact its
    // own deletion observed in the lost lineage. Reconstructing the key from the
    // record's UUIDs is the only way to reach the object, and nothing else in
    // this database hints that retained-source storage was ever in use.
    sqlx::query(
        "INSERT INTO erasure_records (subject_type, subject_id, user_id, had_source) \
         VALUES ('novel', $1, $2, TRUE)",
    )
    .bind(unknown_novel)
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();
    // A novel that never held a retained source: nothing to delete, so replay
    // must never enqueue a speculative key for it.
    sqlx::query(
        "INSERT INTO erasure_records (subject_type, subject_id, user_id) VALUES ('novel', $1, $2)",
    )
    .bind(keyless_novel)
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();
    // A record written before its subject row is deleted — what the disaster
    // restore does for an unlisted novel — so it cannot know had_source yet.
    sqlx::query(
        "INSERT INTO erasure_records (subject_type, subject_id, user_id) VALUES ('novel', $1, $2)",
    )
    .bind(keyed_novel)
    .bind(owner)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(ERASURE_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();

    let novels: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM novels")
        .fetch_one(&pool)
        .await
        .unwrap();
    let chapters: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM chapters")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((novels, chapters), (0, 0));
    assert_eq!(
        outbox(&pool).await,
        vec![reconstructed_source.clone(), keyed_source.clone()]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    );
    // The delete raised had_source on the record that was written without it,
    // and the keyless novel neither gained the flag nor a re-queue stamp.
    let mut state = records(&pool).await;
    state.sort();
    assert_eq!(
        state,
        vec![
            ("novel".to_string(), keyed_novel, owner, true, true),
            ("novel".to_string(), keyless_novel, owner, false, false),
            ("novel".to_string(), unknown_novel, owner, true, true),
        ]
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
    );

    // A second deployment replays cleanly: no new re-queue, no row changes. The
    // outbox is self-consuming, so the durable per-record bookkeeping — not the
    // outbox — is what makes the re-queue exactly once per lineage.
    let stamps: Vec<(String, Uuid, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT subject_type, subject_id, source_requeued_at FROM erasure_records ORDER BY 1, 2",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    sqlx::query("DELETE FROM source_file_deletions")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::raw_sql(ERASURE_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();

    assert!(outbox(&pool).await.is_empty());
    assert_eq!(
        sqlx::query_as::<_, (String, Uuid, Option<chrono::DateTime<chrono::Utc>>)>(
            "SELECT subject_type, subject_id, source_requeued_at FROM erasure_records ORDER BY 1, 2",
        )
        .fetch_all(&pool)
        .await
        .unwrap(),
        stamps
    );
}

#[tokio::test]
async fn erasure_replay_preserves_the_final_account_invariant() {
    let pool = scratch_database("novelworld_erasure_final_account").await;
    let owner = Uuid::new_v4();
    let survivor = Uuid::new_v4();
    seed_account(&pool, owner, &[]).await;
    seed_account(&pool, survivor, &[]).await;
    sqlx::query(
        "INSERT INTO runtime_llm_config (provider, api_url, model, api_key_nonce, api_key_ciphertext) \
         VALUES ('deepseek', 'https://provider.invalid', 'model', '\\x00', '\\x00')",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(owner)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(ERASURE_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    // Accounts remain, so the installation keeps its runtime configuration.
    let configured: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM runtime_llm_config")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(configured, 1);

    // Replay that leaves no account clears the configuration, exactly as
    // interactive final-account deletion does, returning the installation to
    // first-run setup.
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(survivor)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO users (id, email, password_hash) VALUES ($1, $2, 'hash')")
        .bind(survivor)
        .bind(format!("erasure-{survivor}@test.invalid"))
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(ERASURE_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    let (users, configured): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(*) FROM users), (SELECT COUNT(*) FROM runtime_llm_config)",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!((users, configured), (0, 0));
}

#[tokio::test]
async fn erasure_journal_drift_fails_novel_service_readiness() {
    let pool = scratch_database("novelworld_erasure_readiness").await;
    let readiness = PgReadinessProbe::new(pool.clone());
    assert!(readiness.is_ready().await);

    for (break_sql, repair_sql) in [
        (
            "DROP TRIGGER record_novel_erasure ON public.novels",
            "CREATE TRIGGER record_novel_erasure AFTER DELETE ON public.novels \
             FOR EACH ROW EXECUTE FUNCTION public.record_novel_erasure()",
        ),
        (
            "DROP TRIGGER record_user_erasure ON public.users",
            "CREATE TRIGGER record_user_erasure AFTER DELETE ON public.users \
             FOR EACH ROW EXECUTE FUNCTION public.record_user_erasure()",
        ),
        (
            "ALTER TABLE public.erasure_records DROP CONSTRAINT erasure_records_pkey",
            "ALTER TABLE public.erasure_records ADD CONSTRAINT erasure_records_pkey \
             PRIMARY KEY (subject_type, subject_id)",
        ),
        (
            "ALTER TABLE public.erasure_records DROP COLUMN source_requeued_at",
            "ALTER TABLE public.erasure_records ADD COLUMN source_requeued_at TIMESTAMPTZ",
        ),
        (
            "ALTER TABLE public.erasure_records DROP COLUMN had_source",
            "ALTER TABLE public.erasure_records ADD COLUMN had_source BOOLEAN NOT NULL DEFAULT FALSE",
        ),
        (
            "ALTER TABLE public.erasure_records \
             DROP CONSTRAINT erasure_records_subject_type_check",
            "ALTER TABLE public.erasure_records \
             ADD CONSTRAINT erasure_records_subject_type_check \
             CHECK (subject_type IN ('user', 'novel'))",
        ),
    ] {
        sqlx::raw_sql(break_sql).execute(&pool).await.unwrap();
        assert!(!readiness.is_ready().await, "drift accepted: {break_sql}");
        sqlx::raw_sql(repair_sql).execute(&pool).await.unwrap();
        assert!(readiness.is_ready().await, "repair rejected: {repair_sql}");
    }

    // A disabled trigger records nothing while still existing by name.
    sqlx::raw_sql("ALTER TABLE public.novels DISABLE TRIGGER record_novel_erasure")
        .execute(&pool)
        .await
        .unwrap();
    assert!(!readiness.is_ready().await);
    sqlx::raw_sql("ALTER TABLE public.novels ENABLE TRIGGER record_novel_erasure")
        .execute(&pool)
        .await
        .unwrap();
    assert!(readiness.is_ready().await);

    // A trigger that fires BEFORE the delete, or once per statement, cannot
    // hold the record in the same transaction as every deleted row.
    sqlx::raw_sql(
        "DROP TRIGGER record_novel_erasure ON public.novels; \
         CREATE TRIGGER record_novel_erasure AFTER DELETE ON public.novels \
         FOR EACH STATEMENT EXECUTE FUNCTION public.record_novel_erasure()",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(!readiness.is_ready().await);

    // Nor does a same-named trigger wired to a decoy function.
    sqlx::raw_sql(
        "CREATE SCHEMA decoy; \
         CREATE FUNCTION decoy.record_novel_erasure() RETURNS TRIGGER LANGUAGE plpgsql \
             AS $decoy$ BEGIN RETURN OLD; END $decoy$; \
         DROP TRIGGER record_novel_erasure ON public.novels; \
         CREATE TRIGGER record_novel_erasure AFTER DELETE ON public.novels \
         FOR EACH ROW EXECUTE FUNCTION decoy.record_novel_erasure()",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(!readiness.is_ready().await);
}

#[tokio::test]
async fn erasure_records_stay_out_of_account_export() {
    let pool = scratch_database("novelworld_erasure_export").await;
    let owner = Uuid::new_v4();
    let exported_novel = Uuid::new_v4();
    let erased_novel = Uuid::new_v4();
    seed_account(
        &pool,
        owner,
        &[(exported_novel, None), (erased_novel, None)],
    )
    .await;
    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(erased_novel)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(records(&pool).await.len(), 1);

    let export = PgAccountExport::new(pool.clone());
    let mut stream = AccountExportPort::export_user(&export, owner);
    let mut kinds = Vec::new();
    let mut body = String::new();
    while let Some(record) = stream.next().await {
        let record = record.unwrap();
        kinds.push(record.kind.to_string());
        body.push_str(&record.data.to_string());
    }
    assert!(kinds.contains(&"novel".to_string()));
    assert!(!kinds.iter().any(|kind| kind.contains("erasure")));
    assert!(body.contains(&exported_novel.to_string()));
    assert!(!body.contains(&erased_novel.to_string()));
    assert!(!body.contains("erasure"));
}

#[tokio::test]
async fn restore_attestations_record_every_required_decision_field() {
    let pool = scratch_database("novelworld_restore_attestations").await;
    let subject = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO restore_attestations \
         (subject_id, decision, window_start, window_end, artifact_inventory, \
          operator_identity, designated_admin) \
         VALUES ($1, 'retain', '2026-08-01 00:00:00+00', '2026-08-02 00:00:00+00', \
                 'sha256:aaa,sha256:bbb', 'operator@example.invalid', TRUE)",
    )
    .bind(subject)
    .execute(&pool)
    .await
    .unwrap();
    let row = sqlx::query(
        "SELECT subject_id, decision, window_start, window_end, artifact_inventory, \
                operator_identity, designated_admin, recorded_at FROM restore_attestations",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.get::<Uuid, _>("subject_id"), subject);
    assert_eq!(row.get::<String, _>("decision"), "retain");
    assert!(
        row.get::<chrono::DateTime<chrono::Utc>, _>("window_start")
            < row.get::<chrono::DateTime<chrono::Utc>, _>("window_end")
    );
    assert_eq!(
        row.get::<String, _>("artifact_inventory"),
        "sha256:aaa,sha256:bbb"
    );
    assert_eq!(
        row.get::<String, _>("operator_identity"),
        "operator@example.invalid"
    );
    assert!(row.get::<bool, _>("designated_admin"));
    let _: chrono::DateTime<chrono::Utc> = row.get("recorded_at");

    // Only the two sanctioned decisions may be recorded.
    let rejected = sqlx::query(
        "INSERT INTO restore_attestations \
         (subject_id, decision, window_start, window_end, artifact_inventory, operator_identity) \
         VALUES ($1, 'maybe', pg_catalog.now(), pg_catalog.now(), 'sha256:aaa', 'operator')",
    )
    .bind(subject)
    .execute(&pool)
    .await;
    assert!(rejected.is_err());
}

/// A restore reloads the text `pg_dump` deparsed, and PostgreSQL re-parses
/// `CHECK (x IN (...))` over a varchar column into an equivalent but differently
/// spelled expression. Every exact-text contract guard has to accept both, or a
/// restored deployment can never migrate or reach readiness again.
#[tokio::test]
async fn restored_constraint_spelling_still_migrates_and_reaches_readiness() {
    let pool = scratch_database("novelworld_restored_spelling").await;
    let novel_readiness = PgReadinessProbe::new(pool.clone());
    let narrative_readiness = NarrativePgReadinessProbe::new(pool.clone());
    assert!(novel_readiness.is_ready().await);
    assert!(NarrativeReadinessPort::is_ready(&narrative_readiness).await);

    sqlx::raw_sql(
        "ALTER TABLE public.chat_turns DROP CONSTRAINT chat_turns_status_check, \
         ADD CONSTRAINT chat_turns_status_check CHECK (((status)::text = ANY (ARRAY[\
             ('in_progress'::character varying)::text, \
             ('completed'::character varying)::text, \
             ('failed'::character varying)::text]))); \
         ALTER TABLE public.world_turns DROP CONSTRAINT world_turns_status_check, \
         ADD CONSTRAINT world_turns_status_check CHECK (((status)::text = ANY (ARRAY[\
             ('in_progress'::character varying)::text, \
             ('completed'::character varying)::text, \
             ('failed'::character varying)::text]))); \
         ALTER TABLE public.novel_import_jobs DROP CONSTRAINT novel_import_jobs_stage_check, \
         ADD CONSTRAINT novel_import_jobs_stage_check CHECK (((stage)::text = ANY (ARRAY[\
             ('source'::character varying)::text, \
             ('chapters'::character varying)::text, \
             ('enriched'::character varying)::text, \
             ('completed'::character varying)::text]))); \
         DROP INDEX public.idx_novel_import_jobs_recoverable; \
         CREATE INDEX idx_novel_import_jobs_recoverable \
             ON public.novel_import_jobs USING btree (status, lease_expires_at, created_at) \
             WHERE ((status)::text = ANY (ARRAY[('pending'::character varying)::text, \
                                                ('in_progress'::character varying)::text]))",
    )
    .execute(&pool)
    .await
    .unwrap();

    sqlx::raw_sql(CHAT_TURN_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::raw_sql(ERASURE_MIGRATION)
        .execute(&pool)
        .await
        .unwrap();
    assert!(novel_readiness.is_ready().await);
    assert!(NarrativeReadinessPort::is_ready(&narrative_readiness).await);

    // Tolerating the restored spelling must not tolerate a different constraint.
    sqlx::raw_sql(
        "ALTER TABLE public.novel_import_jobs DROP CONSTRAINT novel_import_jobs_stage_check, \
         ADD CONSTRAINT novel_import_jobs_stage_check CHECK (((stage)::text = ANY (ARRAY[\
             ('source'::character varying)::text, \
             ('chapters'::character varying)::text, \
             ('enriched'::character varying)::text, \
             ('completed'::character varying)::text, \
             ('anything'::character varying)::text])))",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(!novel_readiness.is_ready().await);
    sqlx::raw_sql(
        "ALTER TABLE public.world_turns DROP CONSTRAINT world_turns_status_check, \
         ADD CONSTRAINT world_turns_status_check CHECK (((status)::text = ANY (ARRAY[\
             ('in_progress'::character varying)::text, \
             ('completed'::character varying)::text])))",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(!NarrativeReadinessPort::is_ready(&narrative_readiness).await);
}
