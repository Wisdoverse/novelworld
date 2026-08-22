use std::collections::{BTreeMap, HashSet};

use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::{
    entities::canon_story_model::{CanonStoryContent, CanonStoryModel},
    repositories::{CanonExtractionCheckpoint, CanonStoryModelRepository},
};

pub struct PgCanonStoryModelRepository {
    pool: PgPool,
}

impl PgCanonStoryModelRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

async fn validate_model_source(
    transaction: &mut Transaction<'_, Postgres>,
    model: &CanonStoryModel,
) -> Result<()> {
    let novel = sqlx::query_as::<_, SourceNovelRow>(
        "SELECT status::text, total_chapters FROM novels WHERE id = $1",
    )
    .bind(model.novel_id)
    .fetch_optional(&mut **transaction)
    .await?
    .context("canon story model source novel does not exist")?;
    let chapters = sqlx::query_as::<_, ChapterSourceRow>(
        "SELECT chapter_number, content FROM chapters WHERE novel_id = $1",
    )
    .bind(model.novel_id)
    .fetch_all(&mut **transaction)
    .await?
    .into_iter()
    .map(|chapter| (chapter.chapter_number, chapter.content))
    .collect::<BTreeMap<_, _>>();
    let character_ids =
        sqlx::query_scalar::<_, Uuid>("SELECT id FROM characters WHERE novel_id = $1")
            .bind(model.novel_id)
            .fetch_all(&mut **transaction)
            .await?
            .into_iter()
            .collect::<HashSet<_>>();
    if !matches!(novel.status.as_str(), "parsing" | "ready")
        || novel.total_chapters != chapters.len() as i32
    {
        anyhow::bail!(
            "canon story models require a fully enriched novel with every chapter persisted"
        );
    }
    model
        .validate(&chapters, &character_ids)
        .context("canon story model failed source validation")
}

async fn insert_model(
    transaction: &mut Transaction<'_, Postgres>,
    model: &CanonStoryModel,
) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO canon_story_models (
            id, novel_id, model_version, schema_version,
            prompt_version, content, created_at
        ) VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
    )
    .bind(model.id)
    .bind(model.novel_id)
    .bind(model.model_version)
    .bind(model.schema_version)
    .bind(&model.prompt_version)
    .bind(serde_json::to_value(&model.content)?)
    .bind(model.created_at)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

#[async_trait]
impl CanonStoryModelRepository for PgCanonStoryModelRepository {
    async fn find_import_checkpoint(
        &self,
        novel_id: Uuid,
        model_version: i32,
        prompt_version: &str,
        chapter_number: i32,
        chunk_index: i32,
        source_content: &str,
    ) -> Result<Option<String>> {
        let extraction = sqlx::query_scalar::<_, serde_json::Value>(
            r#"SELECT extraction
               FROM canon_extraction_checkpoints
               WHERE novel_id = $1 AND model_version = $2
                 AND prompt_version = $3 AND chapter_number = $4
                 AND chunk_index = $5 AND source_content = $6"#,
        )
        .bind(novel_id)
        .bind(model_version)
        .bind(prompt_version)
        .bind(chapter_number)
        .bind(chunk_index)
        .bind(source_content)
        .fetch_optional(&self.pool)
        .await?;
        extraction
            .map(|value| serde_json::to_string(&value).map_err(Into::into))
            .transpose()
    }

    async fn save_import_checkpoint(
        &self,
        checkpoint: CanonExtractionCheckpoint<'_>,
        attempt: i64,
    ) -> Result<bool> {
        let extraction = serde_json::from_str::<serde_json::Value>(checkpoint.extraction_json)
            .context("canon checkpoint extraction is invalid JSON")?;
        let mut transaction = self.pool.begin().await?;
        let fenced = sqlx::query_scalar::<_, bool>(
            "SELECT TRUE FROM novel_import_jobs \
             WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress' \
               AND stage = 'enriched' \
             FOR UPDATE",
        )
        .bind(checkpoint.novel_id)
        .bind(attempt)
        .fetch_optional(&mut *transaction)
        .await?
        .unwrap_or(false);
        if !fenced {
            return Ok(false);
        }
        sqlx::query(
            r#"INSERT INTO canon_extraction_checkpoints (
                   novel_id, model_version, prompt_version, chapter_number,
                   chunk_index, is_final, source_content, extraction
               ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
               ON CONFLICT (novel_id, model_version, prompt_version, chapter_number, chunk_index)
               DO UPDATE SET is_final = EXCLUDED.is_final,
                             source_content = EXCLUDED.source_content,
                             extraction = EXCLUDED.extraction,
                             updated_at = NOW()"#,
        )
        .bind(checkpoint.novel_id)
        .bind(checkpoint.model_version)
        .bind(checkpoint.prompt_version)
        .bind(checkpoint.chapter_number)
        .bind(checkpoint.chunk_index)
        .bind(checkpoint.is_final)
        .bind(checkpoint.source_content)
        .bind(extraction)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(true)
    }

    async fn insert_import(&self, model: &CanonStoryModel, attempt: i64) -> Result<bool> {
        let mut transaction = self.pool.begin().await?;
        let fenced = sqlx::query_scalar::<_, bool>(
            "SELECT TRUE FROM novel_import_jobs \
             WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress' \
               AND stage = 'enriched' \
             FOR UPDATE",
        )
        .bind(model.novel_id)
        .bind(attempt)
        .fetch_optional(&mut *transaction)
        .await?
        .unwrap_or(false);
        if !fenced {
            return Ok(false);
        }
        validate_model_source(&mut transaction, model).await?;
        insert_model(&mut transaction, model).await?;
        sqlx::query("DELETE FROM canon_extraction_checkpoints WHERE novel_id = $1")
            .bind(model.novel_id)
            .execute(&mut *transaction)
            .await?;
        transaction.commit().await?;
        Ok(true)
    }

    async fn find_version(
        &self,
        novel_id: Uuid,
        model_version: i32,
    ) -> Result<Option<CanonStoryModel>> {
        let row = sqlx::query_as::<_, CanonStoryModelRow>(
            r#"SELECT id, novel_id, model_version, schema_version,
                      prompt_version, content, created_at
               FROM canon_story_models
               WHERE novel_id = $1 AND model_version = $2"#,
        )
        .bind(novel_id)
        .bind(model_version)
        .fetch_optional(&self.pool)
        .await?;
        row.map(CanonStoryModelRow::into_domain).transpose()
    }

    async fn find_latest(&self, novel_id: Uuid) -> Result<Option<CanonStoryModel>> {
        let row = sqlx::query_as::<_, CanonStoryModelRow>(
            r#"SELECT id, novel_id, model_version, schema_version,
                      prompt_version, content, created_at
               FROM canon_story_models
               WHERE novel_id = $1
               ORDER BY model_version DESC
               LIMIT 1"#,
        )
        .bind(novel_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(CanonStoryModelRow::into_domain).transpose()
    }
}

#[derive(sqlx::FromRow)]
struct SourceNovelRow {
    status: String,
    total_chapters: i32,
}

#[derive(sqlx::FromRow)]
struct ChapterSourceRow {
    chapter_number: i32,
    content: String,
}

#[derive(sqlx::FromRow)]
struct CanonStoryModelRow {
    id: Uuid,
    novel_id: Uuid,
    model_version: i32,
    schema_version: i32,
    prompt_version: String,
    content: serde_json::Value,
    created_at: chrono::DateTime<chrono::Utc>,
}

impl CanonStoryModelRow {
    fn into_domain(self) -> Result<CanonStoryModel> {
        let content = serde_json::from_value::<CanonStoryContent>(self.content)
            .context("persisted canon story model content is invalid")?;
        Ok(CanonStoryModel {
            id: self.id,
            novel_id: self.novel_id,
            model_version: self.model_version,
            schema_version: self.schema_version,
            prompt_version: self.prompt_version,
            content,
            created_at: self.created_at,
        })
    }
}
