use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use user_service::{
    application::handlers::{AuthError, AuthHandler, LlmSettingsScope},
    domain::{
        entities::{
            runtime_config::RuntimeLlmConfig,
            user::{RefreshToken, User, UserRole},
        },
        ports::PrivacyCleanupPort,
        repositories::{AccountDeletion, UserRepository, UserSave},
    },
    infrastructure::{
        auth::{jwt::JwtService, password::BcryptPasswordHasher},
        llm::LlmClientTester,
        persistence::pg_user_repo::PgUserRepository,
    },
};
use uuid::Uuid;

const FRESH_SCHEMA: &str = include_str!("../../../infra/postgres/init.sql");
const CONFIG_KEY: &str = "abababababababababababababababababababababababababababababababab";

#[derive(Default)]
struct RecordingPrivacyCleanup {
    calls: AtomicUsize,
    allow_calls: AtomicUsize,
    fail: bool,
}

#[async_trait::async_trait]
impl PrivacyCleanupPort for RecordingPrivacyCleanup {
    async fn clear_user(&self, _user_id: Uuid) -> anyhow::Result<()> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.fail {
            anyhow::bail!("agent cleanup unavailable");
        }
        Ok(())
    }

    async fn allow_user(&self, _user_id: Uuid) -> anyhow::Result<()> {
        self.allow_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

fn db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://test:test@localhost:25432/novelworld_test".into())
}

#[tokio::test]
async fn initial_admin_is_durable_and_single_winner() {
    let admin = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url())
        .await
        .unwrap();
    sqlx::query("DROP DATABASE IF EXISTS novelworld_setup_contract WITH (FORCE)")
        .execute(&admin)
        .await
        .unwrap();
    sqlx::query("CREATE DATABASE novelworld_setup_contract")
        .execute(&admin)
        .await
        .unwrap();

    let options = PgConnectOptions::from_str(&db_url())
        .unwrap()
        .database("novelworld_setup_contract");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql(FRESH_SCHEMA).execute(&pool).await.unwrap();

    let empty_handler = AuthHandler {
        user_repo: Arc::new(PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap()),
        jwt: Arc::new(JwtService::new("setup-contract-secret-is-long-enough", 60)),
        llm_tester: Arc::new(LlmClientTester),
        privacy_cleanup: Arc::new(RecordingPrivacyCleanup::default()),
        environment_llm_config: None,
        refresh_token_expiry: 60,
        password_hasher: Arc::new(BcryptPasswordHasher::new(0)),
    };
    assert!(matches!(
        empty_handler
            .register("ordinary@test.invalid", "password123", None)
            .await,
        Err(AuthError::SetupRequired)
    ));

    let left_user = User::new_admin(
        "first@test.invalid".into(),
        "test-hash".into(),
        Some("First".into()),
    );
    let right_user = User::new_admin(
        "second@test.invalid".into(),
        "test-hash".into(),
        Some("Second".into()),
    );
    let left_token = RefreshToken::new(left_user.id, "a".repeat(64), 60);
    let right_token = RefreshToken::new(right_user.id, "b".repeat(64), 60);
    let left_repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    let right_repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    let (left, right) = tokio::join!(
        left_repo.save_initial_setup(&left_user, &left_token),
        right_repo.save_initial_setup(&right_user, &right_token)
    );
    let left_won = left.unwrap();
    assert_ne!(left_won, right.unwrap());

    let restarted_repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    assert!(restarted_repo.has_any().await.unwrap());
    assert!(restarted_repo
        .find_runtime_llm_config()
        .await
        .unwrap()
        .is_none());
    let winner: (Uuid, String) = sqlx::query_as("SELECT id, role::text FROM users")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(winner.1, UserRole::Admin.as_str());
    let token_owner: Uuid = sqlx::query_scalar("SELECT user_id FROM refresh_tokens")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(token_owner, winner.0);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert!(matches!(
        empty_handler
            .register("capacity@test.invalid", "password123", None)
            .await,
        Err(AuthError::Capacity)
    ));

    pool.close().await;
    sqlx::query("DROP DATABASE novelworld_setup_contract WITH (FORCE)")
        .execute(&admin)
        .await
        .unwrap();
}

#[tokio::test]
async fn account_erasure_fails_closed_cascades_owned_data_and_resets_final_setup() {
    let admin_pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(&db_url())
        .await
        .unwrap();
    sqlx::query("DROP DATABASE IF EXISTS novelworld_erasure_contract WITH (FORCE)")
        .execute(&admin_pool)
        .await
        .unwrap();
    sqlx::query("CREATE DATABASE novelworld_erasure_contract")
        .execute(&admin_pool)
        .await
        .unwrap();

    let options = PgConnectOptions::from_str(&db_url())
        .unwrap()
        .database("novelworld_erasure_contract");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect_with(options)
        .await
        .unwrap();
    sqlx::raw_sql(FRESH_SCHEMA).execute(&pool).await.unwrap();

    let repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    let admin = User::new_admin(
        "privacy-admin@test.invalid".into(),
        "hash".into(),
        Some("Privacy Admin".into()),
    );
    let target = User::new(
        "privacy-target@test.invalid".into(),
        "hash".into(),
        Some("Privacy Target".into()),
    );
    let remaining = User::new("privacy-remaining@test.invalid".into(), "hash".into(), None);
    assert_eq!(repo.save(&admin).await.unwrap(), UserSave::Saved);
    assert_eq!(repo.save(&target).await.unwrap(), UserSave::Saved);
    assert_eq!(repo.save(&remaining).await.unwrap(), UserSave::Saved);
    repo.save_runtime_llm_config(
        &RuntimeLlmConfig::for_settings("deepseek", "deepseek-v4-flash", "privacy-secret", false)
            .unwrap(),
    )
    .await
    .unwrap();
    repo.save_user_llm_config(
        target.id,
        &RuntimeLlmConfig::for_settings("openai", "gpt-4o-mini", "target-secret", false).unwrap(),
    )
    .await
    .unwrap();

    let novel_id = Uuid::new_v4();
    let chapter_id = Uuid::new_v4();
    let character_id = Uuid::new_v4();
    let other_character_id = Uuid::new_v4();
    let turn_id = Uuid::new_v4();
    let node_id = Uuid::new_v4();
    sqlx::query("INSERT INTO novels (id, user_id, title, original_file_key, status) VALUES ($1, $2, 'Private novel', 'source.txt', 'ready')")
        .bind(novel_id)
        .bind(target.id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO chapters (id, novel_id, chapter_number, content) VALUES ($1, $2, 1, 'private source text')")
        .bind(chapter_id)
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO chapter_chunks (chapter_id, chunk_index, content) VALUES ($1, 0, 'private source chunk')")
        .bind(chapter_id)
        .execute(&pool)
        .await
        .unwrap();
    for (id, name) in [
        (character_id, "Private Hero"),
        (other_character_id, "Private Friend"),
    ] {
        sqlx::query("INSERT INTO characters (id, novel_id, name, avatar_url, avatar_status) VALUES ($1, $2, $3, 'https://provider.invalid/private.png', 'ready')")
            .bind(id)
            .bind(novel_id)
            .bind(name)
            .execute(&pool)
            .await
            .unwrap();
    }
    sqlx::query("INSERT INTO character_relationships (novel_id, from_character_id, to_character_id, relationship_type) VALUES ($1, $2, $3, 'friend')")
        .bind(novel_id)
        .bind(character_id)
        .bind(other_character_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO character_memories (character_id, user_id, novel_id, layer, content) VALUES ($1, $2, $3, 'permanent', 'private memory')")
        .bind(character_id)
        .bind(target.id)
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO chat_turns (id, user_id, character_id, novel_id, request_fingerprint, chapter_context, reader_identity_type, deviation_mode, status, lease_expires_at) VALUES ($1, $2, $3, $4, $5, 1, 'self', 'canon', 'in_progress', NOW() + INTERVAL '1 minute')")
        .bind(turn_id)
        .bind(target.id)
        .bind(character_id)
        .bind(novel_id)
        .bind(vec![7_u8; 32])
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO chat_messages (character_id, user_id, novel_id, role, content, chapter_context, turn_id) VALUES ($1, $2, $3, 'user', 'private message', 1, $4)")
        .bind(character_id)
        .bind(target.id)
        .bind(novel_id)
        .bind(turn_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO narrative_nodes (id, user_id, novel_id, chapter_number, description, choices) VALUES ($1, $2, $3, 1, 'private node', '[]')")
        .bind(node_id)
        .bind(target.id)
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    let transition = serde_json::json!({
        "schema_version": 1,
        "prompt_version": "test-v1",
        "canon_model_version": 1,
        "canonical_checkpoint_chapter": 1,
        "rendered_narrative": "private consequence",
        "events": [],
        "relationship_changes": [],
        "location_changes": [],
        "thread_changes": []
    });
    sqlx::query("INSERT INTO user_choices (user_id, novel_id, node_id, chapter_number, choice_index, choice_text, consequence, transition) VALUES ($1, $2, $3, 1, 0, 'private choice', 'private consequence', $4)")
        .bind(target.id)
        .bind(novel_id)
        .bind(node_id)
        .bind(transition)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO world_states (user_id, novel_id, state) VALUES ($1, $2, '{\"private\":true}')",
    )
    .bind(target.id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("INSERT INTO user_novels (user_id, novel_id) VALUES ($1, $2)")
        .bind(target.id)
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO player_chapters (user_id, novel_id, chapter_number, content, origin) VALUES ($1, $2, 1, 'private generated prose', 'choice')")
        .bind(target.id)
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO canon_story_models (novel_id, model_version, schema_version, prompt_version, content) VALUES ($1, 1, 1, 'test-v1', '{\"private\":true}')")
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO reading_progress (user_id, novel_id) VALUES ($1, $2)")
        .bind(target.id)
        .bind(novel_id)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO refresh_tokens (user_id, token, expires_at) VALUES ($1, $2, NOW() + INTERVAL '1 day')")
        .bind(target.id)
        .bind("privacy-refresh-token")
        .execute(&pool)
        .await
        .unwrap();

    let last_admin_cleanup = Arc::new(RecordingPrivacyCleanup::default());
    let handler = |privacy_cleanup: Arc<dyn PrivacyCleanupPort>| AuthHandler {
        user_repo: Arc::new(PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap()),
        jwt: Arc::new(JwtService::new(
            "privacy-contract-secret-is-long-enough",
            60,
        )),
        llm_tester: Arc::new(LlmClientTester),
        privacy_cleanup,
        environment_llm_config: None,
        refresh_token_expiry: 60,
        password_hasher: Arc::new(BcryptPasswordHasher::new(2)),
    };
    let admin_settings = handler(Arc::new(RecordingPrivacyCleanup::default()))
        .llm_settings(admin.id)
        .await
        .unwrap();
    assert_eq!(admin_settings.scope, LlmSettingsScope::Platform);
    assert_eq!(admin_settings.config.api_key, "privacy-secret");
    let target_settings = handler(Arc::new(RecordingPrivacyCleanup::default()))
        .llm_settings(target.id)
        .await
        .unwrap();
    assert_eq!(target_settings.scope, LlmSettingsScope::User);
    assert_eq!(target_settings.config.api_key, "target-secret");
    let fallback_settings = handler(Arc::new(RecordingPrivacyCleanup::default()))
        .llm_settings(remaining.id)
        .await
        .unwrap();
    assert_eq!(fallback_settings.scope, LlmSettingsScope::Platform);
    assert!(!fallback_settings.api_key_configured);
    assert_eq!(
        handler(Arc::new(RecordingPrivacyCleanup::default()))
            .runtime_llm_config_for(Some(admin.id))
            .await
            .unwrap()
            .api_key,
        "privacy-secret"
    );
    assert!(matches!(
        handler(Arc::new(RecordingPrivacyCleanup::default()))
            .runtime_llm_config_for(Some(Uuid::new_v4()))
            .await,
        Err(AuthError::NotFound)
    ));
    assert!(matches!(
        handler(last_admin_cleanup.clone())
            .delete_account(admin.id)
            .await,
        Err(AuthError::LastAdministrator)
    ));
    assert_eq!(last_admin_cleanup.calls.load(Ordering::SeqCst), 1);
    assert_eq!(last_admin_cleanup.allow_calls.load(Ordering::SeqCst), 1);
    assert!(repo.find_by_id(admin.id).await.unwrap().is_some());

    let backup_admin = User::new_admin(
        "privacy-backup-admin@test.invalid".into(),
        "hash".into(),
        None,
    );
    assert_eq!(repo.save(&backup_admin).await.unwrap(), UserSave::Saved);
    let left_repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    let right_repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    let (left, right) = tokio::join!(
        left_repo.delete_account(admin.id),
        right_repo.delete_account(backup_admin.id),
    );
    let left = left.unwrap();
    let right = right.unwrap();
    assert!(matches!(
        (left, right),
        (AccountDeletion::Deleted, AccountDeletion::LastAdministrator)
            | (AccountDeletion::LastAdministrator, AccountDeletion::Deleted)
    ));
    let surviving_admin_id = if left == AccountDeletion::LastAdministrator {
        admin.id
    } else {
        backup_admin.id
    };

    let failed_cleanup = Arc::new(RecordingPrivacyCleanup {
        calls: AtomicUsize::new(0),
        allow_calls: AtomicUsize::new(0),
        fail: true,
    });
    assert!(matches!(
        handler(failed_cleanup.clone())
            .delete_account(target.id)
            .await,
        Err(AuthError::PrivacyCleanupUnavailable)
    ));
    assert_eq!(failed_cleanup.calls.load(Ordering::SeqCst), 1);
    assert_eq!(failed_cleanup.allow_calls.load(Ordering::SeqCst), 1);
    assert!(repo.find_by_id(target.id).await.unwrap().is_some());

    let successful_cleanup = Arc::new(RecordingPrivacyCleanup::default());
    handler(successful_cleanup.clone())
        .delete_account(target.id)
        .await
        .unwrap();
    assert_eq!(successful_cleanup.calls.load(Ordering::SeqCst), 1);
    assert!(repo.find_by_id(target.id).await.unwrap().is_none());
    assert!(repo
        .find_user_llm_config(target.id)
        .await
        .unwrap()
        .is_none());
    let canonical: serde_json::Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
            'novels', (SELECT COUNT(*) FROM novels),
            'chapters', (SELECT COUNT(*) FROM chapters),
            'chapter_chunks', (SELECT COUNT(*) FROM chapter_chunks),
            'characters', (SELECT COUNT(*) FROM characters),
            'character_relationships', (SELECT COUNT(*) FROM character_relationships),
            'canon_story_models', (SELECT COUNT(*) FROM canon_story_models)
        )"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for (table, expected) in [
        ("novels", 1),
        ("chapters", 1),
        ("chapter_chunks", 1),
        ("characters", 2),
        ("character_relationships", 1),
        ("canon_story_models", 1),
    ] {
        assert_eq!(
            canonical[table].as_i64(),
            Some(expected),
            "{table} was not shared"
        );
    }
    let retained_private: serde_json::Value = sqlx::query_scalar(
        r#"SELECT jsonb_build_object(
            'user_novels', (SELECT COUNT(*) FROM user_novels),
            'character_memories', (SELECT COUNT(*) FROM character_memories),
            'chat_turns', (SELECT COUNT(*) FROM chat_turns),
            'chat_messages', (SELECT COUNT(*) FROM chat_messages),
            'narrative_nodes', (SELECT COUNT(*) FROM narrative_nodes),
            'user_choices', (SELECT COUNT(*) FROM user_choices),
            'world_states', (SELECT COUNT(*) FROM world_states),
            'player_chapters', (SELECT COUNT(*) FROM player_chapters),
            'reading_progress', (SELECT COUNT(*) FROM reading_progress),
            'refresh_tokens', (SELECT COUNT(*) FROM refresh_tokens),
            'user_llm_configs', (SELECT COUNT(*) FROM user_llm_configs)
        )"#,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    for (table, count) in retained_private.as_object().unwrap() {
        assert_eq!(count.as_i64(), Some(0), "{table} retained erased data");
    }

    assert_eq!(
        repo.delete_account(remaining.id).await.unwrap(),
        AccountDeletion::Deleted
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM runtime_llm_config")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    let racing_user = User::new(
        "privacy-racing-user@test.invalid".into(),
        "hash".into(),
        None,
    );
    let delete_repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    let register_repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    let (deleted, registered) = tokio::join!(
        delete_repo.delete_account(surviving_admin_id),
        register_repo.save(&racing_user),
    );
    let deleted = deleted.unwrap();
    let registered = registered.unwrap();
    assert!(matches!(
        (deleted, registered),
        (AccountDeletion::Deleted, UserSave::SetupRequired)
            | (AccountDeletion::LastAdministrator, UserSave::Saved)
    ));
    let (users, admins): (i64, i64) =
        sqlx::query_as("SELECT COUNT(*), COUNT(*) FILTER (WHERE role = 'admin') FROM users")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert!(
        users == 0 || admins > 0,
        "registration stranded users without an admin"
    );
    if registered == UserSave::Saved {
        assert_eq!(
            repo.delete_account(racing_user.id).await.unwrap(),
            AccountDeletion::Deleted
        );
        handler(Arc::new(RecordingPrivacyCleanup::default()))
            .delete_account(surviving_admin_id)
            .await
            .unwrap();
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM users")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM runtime_llm_config")
            .fetch_one(&pool)
            .await
            .unwrap(),
        0
    );

    pool.close().await;
    sqlx::query("DROP DATABASE novelworld_erasure_contract WITH (FORCE)")
        .execute(&admin_pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_refresh_token_lifecycle() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    let user_id = Uuid::new_v4();
    let token_str = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());

    // Create user
    sqlx::query("INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'user')")
        .bind(user_id)
        .bind(format!("refresh_test_{}@test.com", user_id))
        .bind("$2b$12$fakehashfakehashfakehashfakehashfakehashfakehashfak")
        .execute(&pool)
        .await
        .unwrap();

    let repo = PgUserRepository::new(pool.clone(), CONFIG_KEY).unwrap();
    repo.save_refresh_token(&RefreshToken::new(user_id, token_str.clone(), 60))
        .await
        .unwrap();
    let left = RefreshToken::new(user_id, "b".repeat(64), 60);
    let right = RefreshToken::new(user_id, "c".repeat(64), 60);
    let (left_won, right_won) = tokio::join!(
        repo.rotate_refresh_token(&token_str, &left),
        repo.rotate_refresh_token(&token_str, &right)
    );
    assert_ne!(left_won.unwrap(), right_won.unwrap());
    assert!(repo.find_refresh_token(&token_str).await.unwrap().is_none());
    let replacements: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM refresh_tokens WHERE token = $1 OR token = $2")
            .bind(&left.token)
            .bind(&right.token)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(replacements, 1);

    // Cleanup
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_reading_progress_upsert() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();

    // Setup
    sqlx::query("INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'user')")
        .bind(user_id)
        .bind(format!("progress_{}@test.com", user_id))
        .bind("$2b$12$fakehashfakehashfakehashfakehashfakehashfakehashfak")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO novels (id, user_id, title, status) VALUES ($1, $2, $3, 'ready')")
        .bind(novel_id)
        .bind(user_id)
        .bind("Progress Test")
        .execute(&pool)
        .await
        .unwrap();

    // Create progress
    sqlx::query(
        "INSERT INTO reading_progress (id, user_id, novel_id, current_chapter, reader_identity_type, deviation_mode)
         VALUES ($1, $2, $3, 1, 'self', 'canon')"
    )
    .bind(Uuid::new_v4()).bind(user_id).bind(novel_id)
    .execute(&pool).await.unwrap();

    // Update chapter
    sqlx::query(
        "UPDATE reading_progress SET current_chapter = 5 WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .execute(&pool)
    .await
    .unwrap();

    let row: (i32,) = sqlx::query_as(
        "SELECT current_chapter FROM reading_progress WHERE user_id = $1 AND novel_id = $2",
    )
    .bind(user_id)
    .bind(novel_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(row.0, 5);

    // Cleanup
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_world_state_jsonb() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();

    // Setup
    sqlx::query("INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'user')")
        .bind(user_id)
        .bind(format!("world_{}@test.com", user_id))
        .bind("$2b$12$fakehashfakehashfakehashfakehashfakehashfakehashfak")
        .execute(&pool)
        .await
        .unwrap();

    sqlx::query("INSERT INTO novels (id, user_id, title, status) VALUES ($1, $2, $3, 'ready')")
        .bind(novel_id)
        .bind(user_id)
        .bind("World Test")
        .execute(&pool)
        .await
        .unwrap();

    // Create world state
    let state = serde_json::json!({
        "choices": [{"chapter": 3, "choice": "Fight", "consequence": "Victory"}],
        "relationships": {"Hero": {"score": 75, "last_change": "saved the day"}},
        "world_events": ["The dragon was defeated"]
    });

    sqlx::query("INSERT INTO world_states (id, user_id, novel_id, state) VALUES ($1, $2, $3, $4)")
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(novel_id)
        .bind(&state)
        .execute(&pool)
        .await
        .unwrap();

    // Query JSONB
    let row: (serde_json::Value,) =
        sqlx::query_as("SELECT state FROM world_states WHERE user_id = $1 AND novel_id = $2")
            .bind(user_id)
            .bind(novel_id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(row.0["relationships"]["Hero"]["score"], 75);
    assert_eq!(row.0["choices"][0]["choice"], "Fight");

    // Cleanup
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();
}
