use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::repositories::{ReadingProgressRepository, ReadingProgressRecord};

pub struct PgReadingProgressRepository {
    pool: PgPool,
}

impl PgReadingProgressRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReadingProgressRepository for PgReadingProgressRepository {
    async fn get_or_create(&self, user_id: Uuid, novel_id: Uuid) -> Result<ReadingProgressRecord> {
        let row = sqlx::query_as::<_, ProgressRow>(
            r#"SELECT id, user_id, novel_id, current_chapter, reader_identity,
                      reader_identity_type::text, reader_character_id,
                      deviation_mode::text, last_read_at, created_at
               FROM reading_progress WHERE user_id = $1 AND novel_id = $2"#
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = row {
            return Ok(r.into());
        }

        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        sqlx::query(
            r#"INSERT INTO reading_progress (id, user_id, novel_id, current_chapter, reader_identity_type, deviation_mode, last_read_at, created_at)
               VALUES ($1, $2, $3, 1, 'self', 'canon', $4, $4)"#
        )
        .bind(id)
        .bind(user_id)
        .bind(novel_id)
        .bind(now)
        .execute(&self.pool)
        .await?;

        Ok(ReadingProgressRecord {
            id,
            user_id,
            novel_id,
            current_chapter: 1,
            reader_identity: None,
            reader_identity_type: "self".to_string(),
            reader_character_id: None,
            deviation_mode: "canon".to_string(),
            last_read_at: now,
            created_at: now,
        })
    }

    async fn update_chapter(&self, user_id: Uuid, novel_id: Uuid, chapter: i32) -> Result<()> {
        self.get_or_create(user_id, novel_id).await?;
        sqlx::query(
            r#"UPDATE reading_progress SET current_chapter = $3, last_read_at = NOW()
               WHERE user_id = $1 AND novel_id = $2"#
        )
        .bind(user_id)
        .bind(novel_id)
        .bind(chapter)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn set_identity(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        identity_type: &str,
        identity_name: Option<&str>,
        character_id: Option<Uuid>,
    ) -> Result<()> {
        self.get_or_create(user_id, novel_id).await?;
        sqlx::query(
            r#"UPDATE reading_progress
               SET reader_identity_type = $3::identity_type,
                   reader_identity = $4,
                   reader_character_id = $5
               WHERE user_id = $1 AND novel_id = $2"#
        )
        .bind(user_id)
        .bind(novel_id)
        .bind(identity_type)
        .bind(identity_name)
        .bind(character_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct ProgressRow {
    id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    current_chapter: i32,
    reader_identity: Option<String>,
    reader_identity_type: String,
    reader_character_id: Option<Uuid>,
    deviation_mode: String,
    last_read_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl From<ProgressRow> for ReadingProgressRecord {
    fn from(r: ProgressRow) -> Self {
        ReadingProgressRecord {
            id: r.id,
            user_id: r.user_id,
            novel_id: r.novel_id,
            current_chapter: r.current_chapter,
            reader_identity: r.reader_identity,
            reader_identity_type: r.reader_identity_type,
            reader_character_id: r.reader_character_id,
            deviation_mode: r.deviation_mode,
            last_read_at: r.last_read_at,
            created_at: r.created_at,
        }
    }
}
