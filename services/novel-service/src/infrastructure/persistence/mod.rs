pub mod account_export;
pub mod canon_story_model_pg_repo;
pub mod chapter_pg_repo;
pub mod character_pg_repo;
pub mod novel_pg_repo;
pub mod pg_progress_repo;
pub mod source_file_deletion_pg_repo;

use crate::domain::ports::ReadinessProbe;
use async_trait::async_trait;
use sqlx::PgPool;
use std::time::Duration;

pub struct PgReadinessProbe {
    pool: PgPool,
}

impl PgReadinessProbe {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReadinessProbe for PgReadinessProbe {
    async fn is_ready(&self) -> bool {
        matches!(
            tokio::time::timeout(
                Duration::from_secs(2),
                sqlx::query("SELECT 1").execute(&self.pool),
            )
            .await,
            Ok(Ok(_))
        )
    }
}
