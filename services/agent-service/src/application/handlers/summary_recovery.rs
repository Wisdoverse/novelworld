use anyhow::Result;
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tracing::Instrument;

use super::{character_is_available, validate_persona_visibility, AgentCommandHandler};
use crate::domain::repositories::{SummaryOutcome, SummaryWindow, SummaryWindowRepository};

impl AgentCommandHandler {
    async fn summary_eligible(&self, window: &SummaryWindow, source_chapter: i32) -> Result<bool> {
        let character = self
            .owned_character(window.character_id, window.user_id, Some(window.novel_id))
            .await?;
        if character.id != window.character_id {
            return Ok(false);
        }
        let reading = self.reading_context_for(&character, window.user_id).await?;
        Ok(reading.reader_identity_type == "self"
            && reading.reader_character_id.is_none()
            && reading.current_chapter >= source_chapter
            && character_is_available(character.first_appearance_chapter, reading.current_chapter)
            && validate_persona_visibility(&character, reading.current_chapter).is_ok())
    }

    /// One bounded pass. Neither chat replay nor a read endpoint calls this.
    pub async fn recover_summary_windows(
        &self,
        repository: &dyn SummaryWindowRepository,
        stop: &watch::Receiver<bool>,
    ) -> Result<()> {
        for candidate in repository.due_windows().await? {
            if *stop.borrow() {
                break;
            }
            let Ok(_admission) = self.try_admit_chat(candidate.user_id) else {
                repository.defer_window(&candidate, false).await?;
                continue;
            };
            let Some(window) = repository.claim_window(&candidate).await? else {
                continue;
            };
            let sources = repository.summary_sources(&window).await?;
            let Ok((chapter, _)) = window.validate_sources(&sources) else {
                repository
                    .fail_summary(&window, SummaryOutcome::SourceInvalid)
                    .await?;
                tracing::warn!(
                    summary_outcome = "source_invalid",
                    "chat summary incomplete"
                );
                continue;
            };
            if !self
                .summary_eligible(&window, chapter)
                .await
                .unwrap_or(false)
                || *stop.borrow()
            {
                repository.defer_window(&window, true).await?;
                continue;
            }
            // An ambiguous ACK is not an unsent classification: ? stops this
            // pass without calling the provider or changing dispatched state.
            if !repository.start_summary_dispatch(&window).await? {
                continue;
            }
            let result = tokio::time::timeout(
                Duration::from_secs(300),
                self.memory_manager.summarize_window(&window, &sources),
            )
            .await;
            let text = match result {
                Ok(Ok(text)) => text,
                _ => {
                    repository
                        .fail_summary(&window, SummaryOutcome::DispatchUnknown)
                        .await?;
                    tracing::warn!(
                        summary_outcome = "dispatch_unknown",
                        "chat summary incomplete"
                    );
                    continue;
                }
            };
            let Ok(memory) = window.memory(text, &sources) else {
                repository
                    .fail_summary(&window, SummaryOutcome::OutputInvalid)
                    .await?;
                tracing::warn!(
                    summary_outcome = "output_invalid",
                    "chat summary incomplete"
                );
                continue;
            };
            if !self
                .summary_eligible(&window, chapter)
                .await
                .unwrap_or(false)
            {
                repository
                    .fail_summary(&window, SummaryOutcome::EligibilityChanged)
                    .await?;
                tracing::warn!(
                    summary_outcome = "eligibility_changed",
                    "chat summary incomplete"
                );
                continue;
            }
            if repository.finish_summary(&window, &memory).await? {
                tracing::info!(summary_outcome = "saved", "chat summary completed");
                // No later scan retries this optional work. Mid is already safe.
                if !matches!(
                    tokio::time::timeout(Duration::from_secs(305), async {
                        if let Some(promoted) = self.memory_manager.promote_summary(&memory).await?
                        {
                            tokio::time::timeout(
                                Duration::from_secs(3),
                                self.memory_manager.memory_repo.save(&promoted),
                            )
                            .await??;
                        }
                        Ok::<(), anyhow::Error>(())
                    })
                    .await,
                    Ok(Ok(()))
                ) {
                    tracing::warn!(
                        summary_outcome = "promotion_unavailable",
                        "chat summary promotion incomplete"
                    );
                }
            }
        }
        Ok(())
    }
}

pub fn spawn_summary_worker(
    handler: Arc<AgentCommandHandler>,
    repository: Arc<dyn SummaryWindowRepository>,
    mut stop: watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // ponytail: one sequential summary worker; add a bounded pool only when
        // measured summary throughput requires more than one active call.
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = stop.changed() => break,
                _ = interval.tick() => {
                    if *stop.borrow() { break; }
                    if handler.recover_summary_windows(repository.as_ref(), &stop).await.is_err() {
                        tracing::warn!(summary_outcome = "recovery_unavailable", "chat summary recovery incomplete");
                    }
                }
            }
        }
    }.instrument(tracing::Span::current()))
}
