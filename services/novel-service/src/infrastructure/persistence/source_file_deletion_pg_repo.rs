use anyhow::{ensure, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::repositories::{PendingSourceFileDeletion, SourceFileDeletionRepository};
use crate::infrastructure::persistence::{SOURCE_DELETE_CLAIM_PREFIX, SOURCE_UPLOAD_PENDING};

pub struct PgSourceFileDeletionRepository {
    pool: PgPool,
}

impl PgSourceFileDeletionRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn storage_required(&self) -> Result<bool> {
        Ok(sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM novels \
             WHERE original_file_key LIKE 'source-files/%' \
             AND octet_length(original_file_key) BETWEEN 1 AND 1024) \
             OR EXISTS (SELECT 1 FROM source_file_deletions)",
        )
        .fetch_one(&self.pool)
        .await?)
    }
}

#[async_trait]
impl SourceFileDeletionRepository for PgSourceFileDeletionRepository {
    async fn reserve_upload(&self, object_key: &str, not_before: DateTime<Utc>) -> Result<()> {
        let result = sqlx::query(
            "INSERT INTO source_file_deletions (object_key, next_attempt_at, last_error) \
             VALUES ($1, $2, $3) ON CONFLICT (object_key) DO NOTHING",
        )
        .bind(object_key)
        .bind(not_before)
        .bind(SOURCE_UPLOAD_PENDING)
        .execute(&self.pool)
        .await?;
        ensure!(
            result.rows_affected() == 1,
            "source file upload key already has a cleanup reservation"
        );
        Ok(())
    }

    async fn enqueue_cleanup(&self, object_key: &str, not_before: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "INSERT INTO source_file_deletions (object_key, next_attempt_at, last_error) \
             VALUES ($1, $2, NULL) \
             ON CONFLICT (object_key) DO UPDATE \
             SET next_attempt_at = LEAST(source_file_deletions.next_attempt_at, EXCLUDED.next_attempt_at), \
                 last_error = NULL",
        )
        .bind(object_key)
        .bind(not_before)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn due(&self, limit: i64) -> Result<Vec<PendingSourceFileDeletion>> {
        let rows = sqlx::query_as::<_, (String, i32, String)>(
            "WITH due AS ( \
                 SELECT object_key FROM source_file_deletions \
                 WHERE next_attempt_at <= NOW() \
                 ORDER BY next_attempt_at, object_key \
                 FOR UPDATE SKIP LOCKED LIMIT $1 \
             ) \
             UPDATE source_file_deletions AS deletion \
             SET last_error = $2 || ':' || public.uuid_generate_v4()::text, \
                 next_attempt_at = NOW() + INTERVAL '5 minutes' \
             FROM due WHERE deletion.object_key = due.object_key \
             RETURNING deletion.object_key, deletion.attempts, deletion.last_error",
        )
        .bind(limit.clamp(1, 100))
        .bind(SOURCE_DELETE_CLAIM_PREFIX)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(object_key, attempts, claim_token)| PendingSourceFileDeletion {
                    object_key,
                    attempts,
                    claim_token,
                },
            )
            .collect())
    }

    async fn complete(&self, object_key: &str, claim_token: &str) -> Result<()> {
        sqlx::query("DELETE FROM source_file_deletions WHERE object_key = $1 AND last_error = $2")
            .bind(object_key)
            .bind(claim_token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn retry(
        &self,
        object_key: &str,
        claim_token: &str,
        error: &str,
        not_before: DateTime<Utc>,
    ) -> Result<()> {
        let error: String = error.chars().take(500).collect();
        sqlx::query(
            "UPDATE source_file_deletions SET attempts = attempts + 1, last_error = $2, \
             next_attempt_at = $3 WHERE object_key = $1 AND last_error = $4",
        )
        .bind(object_key)
        .bind(error)
        .bind(not_before)
        .bind(claim_token)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
