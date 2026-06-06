use async_trait::async_trait;
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::repositories::{ChapterReadRepository, NovelInfo};

#[derive(Debug, FromRow)]
struct ChapterContentRow {
    content: String,
}

#[derive(Debug, FromRow)]
struct NovelInfoRow {
    id: Uuid,
    title: String,
    deviation_mode: String,
    world_summary: Option<String>,
}

pub struct PgChapterReadRepository {
    pool: PgPool,
}

impl PgChapterReadRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChapterReadRepository for PgChapterReadRepository {
    async fn get_chapter_content(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> Result<Option<String>> {
        let row = sqlx::query_as::<_, ChapterContentRow>(
            "SELECT content FROM chapters WHERE novel_id = $1 AND chapter_number = $2",
        )
        .bind(novel_id)
        .bind(chapter_number)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.content))
    }

    async fn get_novel_info(&self, novel_id: Uuid) -> Result<Option<NovelInfo>> {
        let row = sqlx::query_as::<_, NovelInfoRow>(
            "SELECT id, title, deviation_mode, world_summary FROM novels WHERE id = $1",
        )
        .bind(novel_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| NovelInfo {
            id: r.id,
            title: r.title,
            deviation_mode: r.deviation_mode,
            world_summary: r.world_summary,
        }))
    }
}
