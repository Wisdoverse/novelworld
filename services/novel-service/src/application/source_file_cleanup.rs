use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use std::{sync::Arc, time::Duration};
use tokio::{sync::watch, task::JoinHandle};
use tracing::Instrument;

use crate::application::{stop_requested, wake_unless_stopped};
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

    pub fn spawn(self, mut stop: watch::Receiver<bool>) -> JoinHandle<()> {
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                let mut delay = Duration::ZERO;
                while wake_unless_stopped(&mut stop, tokio::time::sleep(delay)).await {
                    if let Err(error) = self.drain_once(&stop).await {
                        tracing::error!(error = ?error, "source file cleanup pass failed");
                    }
                    delay = Duration::from_secs(5);
                }
            }
            .instrument(current_span),
        )
    }

    /// Unprocessed claims after a stop expire and are claimed again later.
    pub async fn drain_once(&self, stop: &watch::Receiver<bool>) -> Result<usize> {
        let pending = self.deletions.due(32).await?;
        let count = pending.len();
        for deletion in pending {
            if stop_requested(stop) {
                break;
            }
            match self.storage.delete(&deletion.object_key).await {
                Ok(()) => {
                    self.deletions
                        .complete(&deletion.object_key, &deletion.claim_token)
                        .await?
                }
                Err(error) => {
                    let not_before = Utc::now() + retry_delay(deletion.attempts);
                    self.deletions
                        .retry(
                            &deletion.object_key,
                            &deletion.claim_token,
                            &error.to_string(),
                            not_before,
                        )
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
    use crate::domain::repositories::PendingSourceFileDeletion;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Mutex,
    };

    #[derive(Default)]
    struct FakeDeletions {
        due_calls: AtomicUsize,
        completed: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl SourceFileDeletionRepository for FakeDeletions {
        async fn reserve_upload(
            &self,
            _key: &str,
            _not_before: chrono::DateTime<Utc>,
        ) -> Result<()> {
            unreachable!("unused test repository method")
        }

        async fn enqueue_cleanup(
            &self,
            _key: &str,
            _not_before: chrono::DateTime<Utc>,
        ) -> Result<()> {
            unreachable!("unused test repository method")
        }

        async fn due(&self, _limit: i64) -> Result<Vec<PendingSourceFileDeletion>> {
            self.due_calls.fetch_add(1, Ordering::SeqCst);
            Ok((0..3)
                .map(|index| PendingSourceFileDeletion {
                    object_key: format!("sources/{index}"),
                    attempts: 0,
                    claim_token: format!("claim-{index}"),
                })
                .collect())
        }

        async fn complete(&self, _key: &str, _claim_token: &str) -> Result<()> {
            self.completed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        async fn retry(
            &self,
            _key: &str,
            _claim_token: &str,
            _error: &str,
            _not_before: chrono::DateTime<Utc>,
        ) -> Result<()> {
            unreachable!("unused test repository method")
        }
    }

    /// Sends the stop signal from inside the first deletion when armed.
    #[derive(Default)]
    struct FakeStorage(Mutex<Option<watch::Sender<bool>>>);

    #[async_trait::async_trait]
    impl SourceFileStorage for FakeStorage {
        async fn put(&self, _key: &str, _data: bytes::Bytes) -> Result<()> {
            unreachable!("unused test storage method")
        }

        async fn get(&self, _key: &str) -> Result<Option<bytes::Bytes>> {
            unreachable!("unused test storage method")
        }

        async fn delete(&self, _key: &str) -> Result<()> {
            if let Some(stop) = self.0.lock().unwrap().take() {
                stop.send(true).unwrap();
            }
            Ok(())
        }
    }

    fn worker(storage: FakeStorage) -> (SourceFileCleanupWorker, Arc<FakeDeletions>) {
        let deletions = Arc::new(FakeDeletions::default());
        (
            SourceFileCleanupWorker::new(Arc::new(storage), deletions.clone()),
            deletions,
        )
    }

    async fn assert_stops(handle: JoinHandle<()>) {
        tokio::time::timeout(Duration::from_secs(60), handle)
            .await
            .expect("cleanup worker did not stop")
            .unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn cleanup_worker_stops_after_the_signal() {
        let (stop, stopped) = watch::channel(false);
        let (worker, deletions) = worker(FakeStorage::default());
        let handle = worker.spawn(stopped);
        tokio::time::sleep(Duration::from_secs(1)).await;
        assert_eq!(deletions.due_calls.load(Ordering::SeqCst), 1);

        stop.send(true).unwrap();
        assert_stops(handle).await;
        assert_eq!(deletions.due_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cleanup_worker_with_a_signalled_receiver_starts_no_pass() {
        let (stop, stopped) = watch::channel(false);
        stop.send(true).unwrap();
        let (worker, deletions) = worker(FakeStorage::default());
        assert_stops(worker.spawn(stopped)).await;
        assert_eq!(deletions.due_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn cleanup_worker_stops_when_the_sender_is_dropped() {
        let (stop, stopped) = watch::channel(false);
        let (worker, deletions) = worker(FakeStorage::default());
        let handle = worker.spawn(stopped);
        tokio::time::sleep(Duration::from_secs(1)).await;

        drop(stop);
        assert_stops(handle).await;
        assert_eq!(deletions.due_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test(start_paused = true)]
    async fn cleanup_pass_stops_between_items() {
        let (stop, stopped) = watch::channel(false);
        let (worker, deletions) = worker(FakeStorage(Mutex::new(Some(stop))));

        worker.drain_once(&stopped).await.unwrap();

        assert_eq!(deletions.completed.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn deletion_retry_backoff_is_bounded() {
        assert_eq!(retry_delay(0), ChronoDuration::seconds(1));
        assert_eq!(retry_delay(4), ChronoDuration::seconds(16));
        assert_eq!(retry_delay(100), ChronoDuration::seconds(256));
    }
}
