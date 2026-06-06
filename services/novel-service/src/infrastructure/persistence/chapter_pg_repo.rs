use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::chapter::Chapter;
use crate::domain::repositories::ChapterRepository;

#[derive(Debug, FromRow)]
struct ChapterRow {
    id: Uuid,
    novel_id: Uuid,
    chapter_number: i32,
    title: Option<String>,
    content: String,
    summary: Option<String>,
    is_key_node: bool,
    key_node_description: Option<String>,
    #[allow(dead_code)]
    word_count: i32,
    created_at: DateTime<Utc>,
}

impl From<ChapterRow> for Chapter {
    fn from(r: ChapterRow) -> Self {
        Chapter {
            id: r.id,
            novel_id: r.novel_id,
            chapter_number: r.chapter_number,
            title: r.title,
            content: r.content,
            summary: r.summary,
            is_key_node: r.is_key_node,
            key_node_description: r.key_node_description,
            created_at: r.created_at,
        }
    }
}

pub struct ChapterPgRepository {
    pool: PgPool,
}

impl ChapterPgRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChapterRepository for ChapterPgRepository {
    async fn save_batch(&self, chapters: &[Chapter]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for ch in chapters {
            let word_count = ch.word_count() as i32;
            sqlx::query(
                r#"
                INSERT INTO chapters (
                    id, novel_id, chapter_number, title, content,
                    summary, is_key_node, key_node_description,
                    word_count, created_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                ON CONFLICT (novel_id, chapter_number) DO UPDATE SET
                    title = EXCLUDED.title,
                    content = EXCLUDED.content,
                    summary = EXCLUDED.summary,
                    is_key_node = EXCLUDED.is_key_node,
                    key_node_description = EXCLUDED.key_node_description,
                    word_count = EXCLUDED.word_count
                "#,
            )
            .bind(ch.id)
            .bind(ch.novel_id)
            .bind(ch.chapter_number)
            .bind(&ch.title)
            .bind(&ch.content)
            .bind(&ch.summary)
            .bind(ch.is_key_node)
            .bind(&ch.key_node_description)
            .bind(word_count)
            .bind(ch.created_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    async fn find_by_novel(&self, novel_id: Uuid) -> Result<Vec<Chapter>> {
        let rows = sqlx::query_as::<_, ChapterRow>(
            "SELECT * FROM chapters WHERE novel_id = $1 ORDER BY chapter_number ASC",
        )
        .bind(novel_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Chapter::from).collect())
    }

    async fn find_by_number(&self, novel_id: Uuid, number: i32) -> Result<Option<Chapter>> {
        let row = sqlx::query_as::<_, ChapterRow>(
            "SELECT * FROM chapters WHERE novel_id = $1 AND chapter_number = $2",
        )
        .bind(novel_id)
        .bind(number)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Chapter::from))
    }

    async fn update(&self, chapter: &Chapter) -> Result<()> {
        let word_count = chapter.word_count() as i32;
        sqlx::query(
            r#"
            UPDATE chapters SET
                title = $2, content = $3, summary = $4,
                is_key_node = $5, key_node_description = $6,
                word_count = $7
            WHERE id = $1
            "#,
        )
        .bind(chapter.id)
        .bind(&chapter.title)
        .bind(&chapter.content)
        .bind(&chapter.summary)
        .bind(chapter.is_key_node)
        .bind(&chapter.key_node_description)
        .bind(word_count)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
