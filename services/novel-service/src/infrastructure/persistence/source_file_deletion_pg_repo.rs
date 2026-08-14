use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::domain::repositories::{PendingSourceFileDeletion, SourceFileDeletionRepository};

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
    async fn enqueue(&self, object_key: &str, not_before: DateTime<Utc>) -> Result<()> {
        sqlx::query(
            "INSERT INTO source_file_deletions (object_key, next_attempt_at) VALUES ($1, $2) \
             ON CONFLICT (object_key) DO UPDATE SET \
             next_attempt_at = LEAST(source_file_deletions.next_attempt_at, EXCLUDED.next_attempt_at)",
        )
        .bind(object_key)
        .bind(not_before)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn due(&self, limit: i64) -> Result<Vec<PendingSourceFileDeletion>> {
        let rows = sqlx::query_as::<_, (String, i32)>(
            "SELECT object_key, attempts FROM source_file_deletions \
             WHERE next_attempt_at <= NOW() ORDER BY next_attempt_at, object_key LIMIT $1",
        )
        .bind(limit.clamp(1, 100))
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(object_key, attempts)| PendingSourceFileDeletion {
                object_key,
                attempts,
            })
            .collect())
    }

    async fn complete(&self, object_key: &str) -> Result<()> {
        sqlx::query("DELETE FROM source_file_deletions WHERE object_key = $1")
            .bind(object_key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn retry(&self, object_key: &str, error: &str, not_before: DateTime<Utc>) -> Result<()> {
        let error: String = error.chars().take(500).collect();
        sqlx::query(
            "UPDATE source_file_deletions SET attempts = attempts + 1, last_error = $2, \
             next_attempt_at = $3 WHERE object_key = $1",
        )
        .bind(object_key)
        .bind(error)
        .bind(not_before)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
