use anyhow::Result;
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::repositories::{ReadingProgressRecord, ReadingProgressRepository};

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
    async fn get_or_create(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        deviation_mode: &str,
    ) -> Result<ReadingProgressRecord> {
        let id = Uuid::new_v4();
        let now = chrono::Utc::now();
        sqlx::query(
            r#"INSERT INTO reading_progress (id, user_id, novel_id, current_chapter, reader_identity_type, deviation_mode, last_read_at, created_at)
               VALUES ($1, $2, $3, 1, 'self', $4::deviation_mode, $5, $5)
               ON CONFLICT (user_id, novel_id) DO NOTHING"#
        )
        .bind(id)
        .bind(user_id)
        .bind(novel_id)
        .bind(deviation_mode)
        .bind(now)
        .execute(&self.pool)
        .await?;

        let row = sqlx::query_as::<_, ProgressRow>(
            r#"SELECT id, user_id, novel_id, current_chapter, reader_identity,
                      reader_identity_type::text, reader_character_id,
                      deviation_mode::text, last_read_at, created_at
               FROM reading_progress WHERE user_id = $1 AND novel_id = $2"#,
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.into())
    }

    async fn update_chapter(&self, user_id: Uuid, novel_id: Uuid, chapter: i32) -> Result<()> {
        let result = sqlx::query(
            r#"WITH locked_progress AS MATERIALIZED (
                   SELECT id, reader_identity_type, reader_character_id
                   FROM reading_progress
                   WHERE user_id = $1 AND novel_id = $2
                   FOR UPDATE
               ),
               decision AS (
                   SELECT progress.id,
                          (progress.reader_identity_type = 'character' AND NOT EXISTS (
                              SELECT 1 FROM characters c
                              WHERE c.id = progress.reader_character_id
                                AND c.novel_id = $2
                                AND c.first_appearance_chapter IS NOT NULL
                                AND c.first_appearance_chapter BETWEEN 1 AND $3
                          )) OR (progress.reader_identity_type = 'self' AND progress.reader_character_id IS NOT NULL)
                          AS reset_identity
                   FROM locked_progress progress
               )
               UPDATE reading_progress rp
               SET current_chapter = $3,
                   last_read_at = NOW(),
                   reader_identity = CASE WHEN decision.reset_identity THEN NULL ELSE rp.reader_identity END,
                   reader_identity_type = CASE WHEN decision.reset_identity THEN 'self'::identity_type ELSE rp.reader_identity_type END,
                   reader_character_id = CASE WHEN decision.reset_identity THEN NULL ELSE rp.reader_character_id END
               FROM decision
               WHERE rp.id = decision.id"#,
        )
        .bind(user_id)
        .bind(novel_id)
        .bind(chapter)
        .execute(&self.pool)
        .await?;
        anyhow::ensure!(result.rows_affected() == 1, "reading progress not found");
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
        let result = sqlx::query(
            r#"UPDATE reading_progress
               SET reader_identity_type = $3::identity_type,
                   reader_identity = $4,
                   reader_character_id = $5
               WHERE user_id = $1 AND novel_id = $2
                 AND (
                     ($3::identity_type = 'self' AND $5::uuid IS NULL)
                     OR ($3::identity_type = 'character' AND EXISTS (
                         SELECT 1 FROM characters c
                         WHERE c.id = $5
                           AND c.novel_id = $2
                           AND c.first_appearance_chapter IS NOT NULL
                           AND c.first_appearance_chapter BETWEEN 1 AND reading_progress.current_chapter
                     ))
                 )"#,
        )
        .bind(user_id)
        .bind(novel_id)
        .bind(identity_type)
        .bind(identity_name)
        .bind(character_id)
        .execute(&self.pool)
        .await?;
        anyhow::ensure!(
            result.rows_affected() == 1,
            "reader identity is no longer valid for the current chapter"
        );
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
