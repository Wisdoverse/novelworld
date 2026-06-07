use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

fn db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .unwrap_or_else(|_| "postgres://test:test@localhost:25432/novelworld_test".into())
}

#[tokio::test]
async fn test_pg_connection() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .expect("Failed to connect to test database");

    let row: (i32,) = sqlx::query_as("SELECT 1")
        .fetch_one(&pool)
        .await
        .expect("Failed to execute query");

    assert_eq!(row.0, 1);
}

#[tokio::test]
async fn test_extensions_installed() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    let exts: Vec<(String,)> = sqlx::query_as(
        "SELECT extname::text FROM pg_extension WHERE extname IN ('uuid-ossp', 'pg_trgm', 'vector')"
    )
    .fetch_all(&pool)
    .await
    .unwrap();

    let names: Vec<&str> = exts.iter().map(|e| e.0.as_str()).collect();
    assert!(names.contains(&"uuid-ossp"), "uuid-ossp not installed");
    assert!(names.contains(&"pg_trgm"), "pg_trgm not installed");
    assert!(names.contains(&"vector"), "pgvector not installed");
}

#[tokio::test]
async fn test_tables_exist() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    let expected_tables = [
        "users", "novels", "chapters", "characters", "character_memories",
        "character_relationships", "chat_messages", "narrative_nodes",
        "user_choices", "world_states", "reading_progress", "refresh_tokens",
    ];

    for table in &expected_tables {
        let exists: (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_name = $1)"
        )
        .bind(table)
        .fetch_one(&pool)
        .await
        .unwrap();

        assert!(exists.0, "Table '{}' does not exist", table);
    }
}

#[tokio::test]
async fn test_user_crud() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    let user_id = Uuid::new_v4();
    let email = format!("test_{}@example.com", user_id);

    // INSERT
    sqlx::query(
        "INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'user')"
    )
    .bind(user_id)
    .bind(&email)
    .bind("$2b$12$fakehashfakehashfakehashfakehashfakehashfakehashfak")
    .execute(&pool)
    .await
    .unwrap();

    // SELECT
    let row: (Uuid, String) = sqlx::query_as(
        "SELECT id, email FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.0, user_id);
    assert_eq!(row.1, email);

    // DELETE
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(user_id)
        .execute(&pool)
        .await
        .unwrap();

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_one(&pool)
        .await
        .unwrap();

    assert_eq!(count.0, 0);
}

#[tokio::test]
async fn test_novel_with_chapters_cascade_delete() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();

    // Create user
    sqlx::query("INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'user')")
        .bind(user_id)
        .bind(format!("cascade_test_{}@example.com", user_id))
        .bind("$2b$12$fakehashfakehashfakehashfakehashfakehashfakehashfak")
        .execute(&pool).await.unwrap();

    // Create novel
    sqlx::query("INSERT INTO novels (id, user_id, title, status) VALUES ($1, $2, $3, 'ready')")
        .bind(novel_id)
        .bind(user_id)
        .bind("Test Novel")
        .execute(&pool).await.unwrap();

    // Create chapters
    for i in 1..=3 {
        sqlx::query("INSERT INTO chapters (id, novel_id, chapter_number, content) VALUES ($1, $2, $3, $4)")
            .bind(Uuid::new_v4())
            .bind(novel_id)
            .bind(i)
            .bind(format!("Chapter {} content", i))
            .execute(&pool).await.unwrap();
    }

    // Verify chapters exist
    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chapters WHERE novel_id = $1")
        .bind(novel_id).fetch_one(&pool).await.unwrap();
    assert_eq!(count.0, 3);

    // Delete novel → chapters should cascade
    sqlx::query("DELETE FROM novels WHERE id = $1")
        .bind(novel_id).execute(&pool).await.unwrap();

    let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM chapters WHERE novel_id = $1")
        .bind(novel_id).fetch_one(&pool).await.unwrap();
    assert_eq!(count.0, 0, "Cascade delete failed — chapters remain");

    // Cleanup
    sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&pool).await.unwrap();
}

#[tokio::test]
async fn test_character_memory_with_pgvector() {
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&db_url())
        .await
        .unwrap();

    let user_id = Uuid::new_v4();
    let novel_id = Uuid::new_v4();
    let char_id = Uuid::new_v4();
    let memory_id = Uuid::new_v4();

    // Setup
    sqlx::query("INSERT INTO users (id, email, password_hash, role) VALUES ($1, $2, $3, 'user')")
        .bind(user_id).bind(format!("mem_test_{}@test.com", user_id))
        .bind("$2b$12$fakehashfakehashfakehashfakehashfakehashfakehashfak")
        .execute(&pool).await.unwrap();

    sqlx::query("INSERT INTO novels (id, user_id, title, status) VALUES ($1, $2, $3, 'ready')")
        .bind(novel_id).bind(user_id).bind("Memory Test Novel")
        .execute(&pool).await.unwrap();

    sqlx::query("INSERT INTO characters (id, novel_id, name, role) VALUES ($1, $2, $3, 'protagonist')")
        .bind(char_id).bind(novel_id).bind("Test Hero")
        .execute(&pool).await.unwrap();

    // Insert memory without embedding
    sqlx::query(
        "INSERT INTO character_memories (id, character_id, user_id, layer, content, importance) VALUES ($1, $2, $3, 'permanent', $4, 10)"
    )
    .bind(memory_id).bind(char_id).bind(user_id).bind("The reader saved the village")
    .execute(&pool).await.unwrap();

    // Query memory
    let row: (String, i16) = sqlx::query_as(
        "SELECT content, importance FROM character_memories WHERE id = $1"
    )
    .bind(memory_id).fetch_one(&pool).await.unwrap();

    assert_eq!(row.0, "The reader saved the village");
    assert_eq!(row.1, 10);

    // Cleanup
    sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(&pool).await.unwrap();
}
