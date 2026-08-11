use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use std::str::FromStr;
use std::sync::Arc;
use user_service::{
    application::handlers::{AuthError, AuthHandler},
    domain::{
        entities::user::{RefreshToken, User, UserRole},
        repositories::UserRepository,
    },
    infrastructure::{auth::jwt::JwtService, persistence::pg_user_repo::PgUserRepository},
};
use uuid::Uuid;

const FRESH_SCHEMA: &str = include_str!("../../../infra/postgres/init.sql");

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
        user_repo: Arc::new(PgUserRepository::new(pool.clone())),
        jwt: Arc::new(JwtService::new("setup-contract-secret-is-long-enough", 60)),
        refresh_token_expiry: 60,
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
    let left_repo = PgUserRepository::new(pool.clone());
    let right_repo = PgUserRepository::new(pool.clone());
    let (left, right) = tokio::join!(
        left_repo.save_initial_user(&left_user, &left_token),
        right_repo.save_initial_user(&right_user, &right_token)
    );
    assert_ne!(left.unwrap(), right.unwrap());

    let restarted_repo = PgUserRepository::new(pool.clone());
    assert!(restarted_repo.has_any().await.unwrap());
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

    pool.close().await;
    sqlx::query("DROP DATABASE novelworld_setup_contract WITH (FORCE)")
        .execute(&admin)
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
    let token_id = Uuid::new_v4();
    let token_str = format!("refresh_{}", Uuid::new_v4());

    // Create user
    sqlx::query("INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'user')")
        .bind(user_id)
        .bind(format!("refresh_test_{}@test.com", user_id))
        .bind("$2b$12$fakehashfakehashfakehashfakehashfakehashfakehashfak")
        .execute(&pool)
        .await
        .unwrap();

    // Create refresh token
    sqlx::query(
        "INSERT INTO refresh_tokens (id, user_id, token, expires_at) VALUES ($1, $2, $3, NOW() + INTERVAL '7 days')"
    )
    .bind(token_id).bind(user_id).bind(&token_str)
    .execute(&pool).await.unwrap();

    // Verify token exists
    let row: (Uuid,) = sqlx::query_as("SELECT user_id FROM refresh_tokens WHERE token = $1")
        .bind(&token_str)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.0, user_id);

    // Delete token (logout)
    sqlx::query("DELETE FROM refresh_tokens WHERE token = $1")
        .bind(&token_str)
        .execute(&pool)
        .await
        .unwrap();

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM refresh_tokens WHERE token = $1")
        .bind(&token_str)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count.0, 0);

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
