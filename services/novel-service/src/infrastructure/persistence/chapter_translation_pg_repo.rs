use anyhow::{Context, Result};
use async_trait::async_trait;
use sqlx::{PgPool, Row};

use crate::domain::repositories::{
    BeginChapterTranslation, ChapterTranslationKey, ChapterTranslationRepository,
};

pub struct PgChapterTranslationRepository {
    pool: PgPool,
}

impl PgChapterTranslationRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChapterTranslationRepository for PgChapterTranslationRepository {
    async fn find_ready(&self, key: ChapterTranslationKey<'_>) -> Result<Option<String>> {
        sqlx::query_scalar::<_, String>(
            r#"SELECT translated_content
               FROM chapter_translations
               WHERE chapter_id = $1 AND source_sha256 = $2 AND profile = $3
                 AND status = 'ready'"#,
        )
        .bind(key.chapter_id)
        .bind(key.source_sha256)
        .bind(key.profile)
        .fetch_optional(&self.pool)
        .await
        .map_err(Into::into)
    }

    async fn begin(&self, key: ChapterTranslationKey<'_>) -> Result<BeginChapterTranslation> {
        let mut transaction = self.pool.begin().await?;
        let inserted = sqlx::query_scalar::<_, i64>(
            r#"INSERT INTO chapter_translations (
                   chapter_id, source_sha256, profile, status, attempt, lease_expires_at
               ) VALUES ($1, $2, $3, 'translating', 1, NOW() + INTERVAL '4 minutes')
               ON CONFLICT (chapter_id, source_sha256, profile) DO NOTHING
               RETURNING attempt"#,
        )
        .bind(key.chapter_id)
        .bind(key.source_sha256)
        .bind(key.profile)
        .fetch_optional(&mut *transaction)
        .await?;
        if let Some(attempt) = inserted {
            transaction.commit().await?;
            return Ok(BeginChapterTranslation::Acquired { attempt });
        }

        let row = sqlx::query(
            r#"SELECT status, translated_content,
                      CASE
                          WHEN status = 'translating' AND lease_expires_at > NOW()
                              THEN GREATEST(
                                  1::BIGINT,
                                  CEIL(EXTRACT(EPOCH FROM lease_expires_at - NOW()))::BIGINT
                              )
                          WHEN status = 'failed' AND retry_after_at > NOW()
                              THEN GREATEST(
                                  1::BIGINT,
                                  CEIL(EXTRACT(EPOCH FROM retry_after_at - NOW()))::BIGINT
                              )
                          ELSE 1::BIGINT
                      END AS retry_after_seconds,
                      (status = 'translating' AND lease_expires_at > NOW())
                          OR (status = 'failed' AND retry_after_at > NOW()) AS blocked
               FROM chapter_translations
               WHERE chapter_id = $1 AND source_sha256 = $2 AND profile = $3
               FOR UPDATE"#,
        )
        .bind(key.chapter_id)
        .bind(key.source_sha256)
        .bind(key.profile)
        .fetch_one(&mut *transaction)
        .await
        .context("chapter translation disappeared while acquiring its lease")?;
        let status = row.try_get::<String, _>("status")?;
        if status == "ready" {
            let content = row
                .try_get::<Option<String>, _>("translated_content")?
                .context("ready chapter translation has no content")?;
            transaction.commit().await?;
            return Ok(BeginChapterTranslation::Ready(content));
        }
        if row.try_get::<bool, _>("blocked")? {
            let retry_after_seconds = row.try_get::<i64, _>("retry_after_seconds")?.max(1) as u64;
            transaction.commit().await?;
            return Ok(BeginChapterTranslation::InProgress {
                retry_after_seconds: retry_after_seconds.min(5),
            });
        }

        let attempt = sqlx::query_scalar::<_, i64>(
            r#"UPDATE chapter_translations
               SET status = 'translating', attempt = attempt + 1,
                   lease_expires_at = NOW() + INTERVAL '4 minutes',
                   retry_after_at = NULL, translated_content = NULL,
                   failure_code = NULL, completed_at = NULL, updated_at = NOW()
               WHERE chapter_id = $1 AND source_sha256 = $2 AND profile = $3
                 AND ((status = 'translating' AND lease_expires_at <= NOW())
                      OR (status = 'failed' AND retry_after_at <= NOW()))
               RETURNING attempt"#,
        )
        .bind(key.chapter_id)
        .bind(key.source_sha256)
        .bind(key.profile)
        .fetch_optional(&mut *transaction)
        .await?
        .context("chapter translation has an invalid lease state")?;
        transaction.commit().await?;
        Ok(BeginChapterTranslation::Acquired { attempt })
    }

    async fn complete(
        &self,
        key: ChapterTranslationKey<'_>,
        attempt: i64,
        translated_content: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"UPDATE chapter_translations
               SET status = 'ready', translated_content = $5,
                   lease_expires_at = NULL, retry_after_at = NULL,
                   failure_code = NULL, completed_at = NOW(), updated_at = NOW()
               WHERE chapter_id = $1 AND source_sha256 = $2 AND profile = $3
                 AND attempt = $4 AND status = 'translating'
                 AND lease_expires_at > NOW()"#,
        )
        .bind(key.chapter_id)
        .bind(key.source_sha256)
        .bind(key.profile)
        .bind(attempt)
        .bind(translated_content)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn fail(
        &self,
        key: ChapterTranslationKey<'_>,
        attempt: i64,
        failure_code: &str,
    ) -> Result<bool> {
        anyhow::ensure!(
            !failure_code.is_empty()
                && failure_code.len() <= 64
                && failure_code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
            "invalid chapter-translation failure code"
        );
        let result = sqlx::query(
            r#"UPDATE chapter_translations
               SET status = 'failed', lease_expires_at = NULL,
                   retry_after_at = NOW() + INTERVAL '5 seconds',
                   failure_code = $5, updated_at = NOW()
               WHERE chapter_id = $1 AND source_sha256 = $2 AND profile = $3
                 AND attempt = $4 AND status = 'translating'"#,
        )
        .bind(key.chapter_id)
        .bind(key.source_sha256)
        .bind(key.profile)
        .bind(attempt)
        .bind(failure_code)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }
}
