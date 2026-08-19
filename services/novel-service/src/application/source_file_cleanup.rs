use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use std::{sync::Arc, time::Duration};
use tracing::Instrument;

use crate::domain::{ports::SourceFileStorage, repositories::SourceFileDeletionRepository};

pub struct SourceFileCleanupWorker {
    storage: Arc<dyn SourceFileStorage>,
    deletions: Arc<dyn SourceFileDeletionRepository>,
}

impl SourceFileCleanupWorker {
    pub fn new(
        storage: Arc<dyn SourceFileStorage>,
        deletions: Arc<dyn SourceFileDeletionRepository>,
    ) -> Self {
        Self { storage, deletions }
    }

    pub fn spawn(self) {
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                loop {
                    if let Err(error) = self.drain_once().await {
                        tracing::error!(error = ?error, "source file cleanup pass failed");
                    }
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
            .instrument(current_span),
        );
    }

    pub async fn drain_once(&self) -> Result<usize> {
        let pending = self.deletions.due(32).await?;
        let count = pending.len();
        for deletion in pending {
            match self.storage.delete(&deletion.object_key).await {
                Ok(()) => self.deletions.complete(&deletion.object_key).await?,
                Err(error) => {
                    let not_before = Utc::now() + retry_delay(deletion.attempts);
                    self.deletions
                        .retry(&deletion.object_key, &error.to_string(), not_before)
                        .await?;
                    tracing::warn!(
                        error = ?error,
                        object_key = %deletion.object_key,
                        "source file deletion will retry"
                    );
                }
            }
        }
        Ok(count)
    }
}

fn retry_delay(attempts: i32) -> ChronoDuration {
    ChronoDuration::seconds(1_i64 << attempts.clamp(0, 8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deletion_retry_backoff_is_bounded() {
        assert_eq!(retry_delay(0), ChronoDuration::seconds(1));
        assert_eq!(retry_delay(4), ChronoDuration::seconds(16));
        assert_eq!(retry_delay(100), ChronoDuration::seconds(256));
    }
}
