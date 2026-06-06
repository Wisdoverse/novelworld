use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::novel::Novel;
use crate::domain::repositories::NovelRepository;
use crate::domain::value_objects::{NovelStatus, DeviationMode};

pub struct NovelPgRepository {
    pool: PgPool,
}

impl NovelPgRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl NovelRepository for NovelPgRepository {
    async fn save(&self, novel: &Novel) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO novels (
                id, user_id, title, author, cover_url, description,
                world_summary, genre, original_file_key, total_chapters,
                status, parse_error, deviation_mode, created_at, updated_at
            ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11::novel_status,$12,$13::deviation_mode,$14,$15)"#
        )
        .bind(novel.id)
        .bind(novel.user_id)
        .bind(&novel.title)
        .bind(&novel.author)
        .bind(&novel.cover_url)
        .bind(&novel.description)
        .bind(&novel.world_summary)
        .bind(&novel.genre)
        .bind(&novel.file_key)
        .bind(novel.total_chapters)
        .bind(novel.status.to_str())
        .bind(&novel.parse_error)
        .bind(novel.deviation_mode.to_str())
        .bind(novel.created_at)
        .bind(novel.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Novel>> {
        let row = sqlx::query_as::<_, NovelRow>(
            "SELECT id, user_id, title, author, cover_url, description, world_summary, genre, original_file_key, total_chapters, status::text, parse_error, deviation_mode::text, created_at, updated_at FROM novels WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| r.into()))
    }

    async fn find_by_user(&self, user_id: Uuid) -> Result<Vec<Novel>> {
        let rows = sqlx::query_as::<_, NovelRow>(
            "SELECT id, user_id, title, author, cover_url, description, world_summary, genre, original_file_key, total_chapters, status::text, parse_error, deviation_mode::text, created_at, updated_at FROM novels WHERE user_id = $1 ORDER BY updated_at DESC"
        )
        .bind(user_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(|r| r.into()).collect())
    }

    async fn update(&self, novel: &Novel) -> Result<()> {
        sqlx::query(
            r#"UPDATE novels SET
                title=$2, author=$3, cover_url=$4, description=$5,
                world_summary=$6, genre=$7, total_chapters=$8,
                status=$9::novel_status, parse_error=$10, deviation_mode=$11::deviation_mode, updated_at=$12
            WHERE id=$1"#
        )
        .bind(novel.id)
        .bind(&novel.title)
        .bind(&novel.author)
        .bind(&novel.cover_url)
        .bind(&novel.description)
        .bind(&novel.world_summary)
        .bind(&novel.genre)
        .bind(novel.total_chapters)
        .bind(novel.status.to_str())
        .bind(&novel.parse_error)
        .bind(novel.deviation_mode.to_str())
        .bind(novel.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn delete(&self, id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM novels WHERE id = $1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct NovelRow {
    id: Uuid,
    user_id: Uuid,
    title: String,
    author: Option<String>,
    cover_url: Option<String>,
    description: Option<String>,
    world_summary: Option<String>,
    genre: Option<String>,
    original_file_key: Option<String>,
    total_chapters: i32,
    status: String,
    parse_error: Option<String>,
    deviation_mode: String,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

impl From<NovelRow> for Novel {
    fn from(r: NovelRow) -> Self {
        Novel {
            id: r.id,
            user_id: r.user_id,
            title: r.title,
            author: r.author,
            cover_url: r.cover_url,
            description: r.description,
            world_summary: r.world_summary,
            genre: r.genre,
            file_key: r.original_file_key,
            total_chapters: r.total_chapters,
            status: NovelStatus::from_str(&r.status),
            parse_error: r.parse_error,
            deviation_mode: DeviationMode::from_str(&r.deviation_mode),
            created_at: r.created_at,
            updated_at: r.updated_at,
            domain_events: vec![],
        }
    }
}

impl NovelStatus {
    pub fn to_str(&self) -> &str {
        match self {
            NovelStatus::Pending => "pending",
            NovelStatus::Parsing => "parsing",
            NovelStatus::Ready => "ready",
            NovelStatus::Error => "error",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "parsing" => Self::Parsing,
            "ready" => Self::Ready,
            "error" => Self::Error,
            _ => Self::Pending,
        }
    }
}

impl DeviationMode {
    pub fn to_str(&self) -> &str {
        match self {
            DeviationMode::Canon => "canon",
            DeviationMode::Creative => "creative",
            DeviationMode::Remix => "remix",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "creative" => Self::Creative,
            "remix" => Self::Remix,
            _ => Self::Canon,
        }
    }
}
