use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use futures::{stream, StreamExt};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore};
use tracing::{error, info, Instrument};
use uuid::Uuid;

use crate::application::commands::ImportNovelCommand;
use crate::domain::entities::{
    chapter::{chapters_are_importable, Chapter},
    character::Character,
    novel::Novel,
};
use crate::domain::ports::{
    DocumentTextExtractor, ImagePort, LlmPort, NovelLlmTask, PrivacyCleanupPort, SourceFileStorage,
};
use crate::domain::repositories::{
    CanonStoryModelRepository, ChapterRepository, CharacterRelationshipRecord, CharacterRepository,
    ImportClaim, LoreExcerpt, NovelRepository, ReadingProgressRecord, ReadingProgressRepository,
    SourceFileDeletionRepository, IMPORT_BUDGET_EXHAUSTED_MESSAGE,
};
use crate::domain::services::{canon_story_extractor, node_detector};
use crate::domain::services::{
    character_extractor::{
        build_chunk_extraction_prompt, build_extraction_prompt, build_representative_sample,
        build_scan_plan, find_first_appearance, json_object_payload, merge_extractions,
        needs_chunk_scan, validate_chunk_extraction, validate_extraction, ChunkExtractionResult,
        ExtractionResult,
    },
    novel_parser::NovelParserService,
};
use crate::domain::value_objects::{ImportStage, NovelStatus, ReaderIdentityType};

pub struct NovelCommandHandler {
    pub novel_repo: Arc<dyn NovelRepository>,
    pub chapter_repo: Arc<dyn ChapterRepository>,
    pub character_repo: Arc<dyn CharacterRepository>,
    pub canon_repo: Arc<dyn CanonStoryModelRepository>,
    pub llm: Arc<dyn LlmPort>,
    pub image_client: Arc<dyn ImagePort>,
    pub privacy_cleanup: Arc<dyn PrivacyCleanupPort>,
    pub source_storage: Option<Arc<dyn SourceFileStorage>>,
    pub source_deletions: Arc<dyn SourceFileDeletionRepository>,
    pub document_extractor: Arc<dyn DocumentTextExtractor>,
    pub import_permits: Arc<Semaphore>,
    pub active_import_users: Arc<Mutex<HashSet<Uuid>>>,
}

struct ImportAdmission {
    _permit: OwnedSemaphorePermit,
    active_users: Arc<Mutex<HashSet<Uuid>>>,
    user_id: Uuid,
}

impl Drop for ImportAdmission {
    fn drop(&mut self) {
        if let Ok(mut users) = self.active_users.lock() {
            users.remove(&self.user_id);
        }
    }
}

fn try_import_admission(
    permits: &Arc<Semaphore>,
    active_users: &Arc<Mutex<HashSet<Uuid>>>,
    user_id: Uuid,
) -> std::result::Result<ImportAdmission, ImportCapacityUnavailable> {
    let permit = permits
        .clone()
        .try_acquire_owned()
        .map_err(|_| ImportCapacityUnavailable)?;
    let mut users = active_users.lock().map_err(|_| ImportCapacityUnavailable)?;
    if !users.insert(user_id) {
        return Err(ImportCapacityUnavailable);
    }
    drop(users);
    Ok(ImportAdmission {
        _permit: permit,
        active_users: active_users.clone(),
        user_id,
    })
}

#[derive(Debug, thiserror::Error)]
#[error("Novel import capacity is busy")]
pub struct ImportCapacityUnavailable;

#[derive(Debug, thiserror::Error)]
#[error("Novel import exceeds the processing budget")]
pub struct ImportBudgetExceeded;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct ImportRetryConflict(pub &'static str);

#[derive(Debug, thiserror::Error)]
#[error("Source file storage is unavailable")]
pub struct SourceFileStorageUnavailable(#[source] pub anyhow::Error);

#[derive(Debug, thiserror::Error)]
#[error("Persisted import chapters cannot be resumed")]
pub struct ImportChaptersUnusable;

#[derive(Debug, thiserror::Error)]
#[error("The retained source file is missing")]
pub struct ImportSourceMissing;

#[derive(Debug, thiserror::Error)]
pub enum NovelDeletionError {
    #[error("Novel not found")]
    NotFound,
    #[error("Novel privacy cleanup is unavailable")]
    PrivacyCleanup(#[source] anyhow::Error),
    #[error("Novel deletion failed")]
    Repository(#[source] anyhow::Error),
}

const MAX_AVATARS_PER_NOVEL: usize = 30;
const MAX_IMPORT_CHAPTERS: usize = 2_048;
const MAX_IMPORT_PROVIDER_CALLS: usize = 640;
const IMPORT_LEASE_HEARTBEAT: Duration = Duration::from_secs(30);
const IMPORT_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Debug, thiserror::Error)]
#[error("Novel import lease was lost")]
struct ImportLeaseLost;

struct ImportLease {
    stop: Option<oneshot::Sender<()>>,
    lost: watch::Receiver<bool>,
}

impl ImportLease {
    fn start(repo: Arc<dyn NovelRepository>, novel_id: Uuid, attempt: i64) -> Self {
        let (stop, mut stopped) = oneshot::channel();
        let (lost, receiver) = watch::channel(false);
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                let mut heartbeat = tokio::time::interval(IMPORT_LEASE_HEARTBEAT);
            heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            heartbeat.tick().await;
            loop {
                tokio::select! {
                    _ = &mut stopped => return,
                    _ = heartbeat.tick() => {
                        match repo.renew_import(novel_id, attempt).await {
                            Ok(true) => {}
                            Ok(false) => {
                                tracing::warn!(%novel_id, attempt, "novel import lease was fenced");
                                let _ = lost.send(true);
                                return;
                            }
                            Err(error) => {
                                tracing::error!(%novel_id, attempt, error = ?error, "novel import lease renewal failed");
                                let _ = lost.send(true);
                                return;
                            }
                        }
                    }
                }
            }
            }
            .instrument(current_span),
        );
        Self {
            stop: Some(stop),
            lost: receiver,
        }
    }

    async fn run<T>(&mut self, operation: impl Future<Output = T>) -> Option<T> {
        if *self.lost.borrow() {
            return None;
        }
        tokio::select! {
            biased;
            result = operation => Some(result),
            _ = self.wait_until_lost() => None,
        }
    }

    async fn wait_until_lost(&mut self) {
        while !*self.lost.borrow() && self.lost.changed().await.is_ok() {}
    }

    fn stop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

impl Drop for ImportLease {
    fn drop(&mut self) {
        self.stop();
    }
}

fn ensure_import_budget(chapters: &[Chapter]) -> std::result::Result<(), ImportBudgetExceeded> {
    if chapters.len() > MAX_IMPORT_CHAPTERS {
        return Err(ImportBudgetExceeded);
    }
    let character_scans = if needs_chunk_scan(chapters) {
        build_scan_plan(chapters).len()
    } else {
        0
    };
    let canon_scans = canon_story_extractor::build_scan_plan(chapters)
        .map_err(|_| ImportBudgetExceeded)?
        .len();
    let calls = 2usize
        .saturating_add(character_scans)
        .saturating_add(canon_scans)
        .saturating_add(MAX_AVATARS_PER_NOVEL);
    (calls <= MAX_IMPORT_PROVIDER_CALLS)
        .then_some(())
        .ok_or(ImportBudgetExceeded)
}

impl NovelCommandHandler {
    fn try_admit_import(
        &self,
        user_id: Uuid,
    ) -> std::result::Result<ImportAdmission, ImportCapacityUnavailable> {
        try_import_admission(&self.import_permits, &self.active_import_users, user_id)
    }

    pub async fn delete_owned_novel(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<(), NovelDeletionError> {
        let owned = self
            .novel_repo
            .find_by_id(novel_id)
            .await
            .map_err(NovelDeletionError::Repository)?
            .is_some_and(|novel| novel.user_id == user_id);
        if !owned {
            return Err(NovelDeletionError::NotFound);
        }
        if let Err(error) = self.privacy_cleanup.clear_novel(user_id, novel_id).await {
            if let Err(rollback_error) = self.privacy_cleanup.allow_novel(user_id, novel_id).await {
                tracing::warn!(error = ?rollback_error, %user_id, %novel_id, "novel privacy cleanup rollback failed");
            }
            return Err(NovelDeletionError::PrivacyCleanup(error));
        }
        if let Err(error) = self.novel_repo.delete(novel_id).await {
            if let Err(rollback_error) = self.privacy_cleanup.allow_novel(user_id, novel_id).await {
                tracing::error!(error = ?rollback_error, %user_id, %novel_id, "novel privacy cleanup rollback failed");
            }
            return Err(NovelDeletionError::Repository(error));
        }
        Ok(())
    }

    pub fn spawn_import_recovery(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let handler = self.clone();
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                let mut interval = tokio::time::interval(IMPORT_RECOVERY_INTERVAL);
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    interval.tick().await;
                    if let Err(error) = handler.recover_imports_once().await {
                        error!(error = ?error, "novel import recovery scan failed");
                    }
                }
            }
            .instrument(current_span),
        )
    }

    async fn recover_imports_once(self: &Arc<Self>) -> Result<()> {
        for candidate in self.novel_repo.recoverable_imports(100).await? {
            let Ok(admission) = self.try_admit_import(candidate.user_id) else {
                continue;
            };
            match self
                .novel_repo
                .claim_import(candidate.novel_id, candidate.user_id)
                .await
            {
                Ok(Some(claim)) => self.spawn_claimed_import(claim, admission),
                Ok(None) => {}
                Err(error) => {
                    error!(
                        error = ?error,
                        novel_id = %candidate.novel_id,
                        "recoverable novel import claim failed"
                    );
                }
            }
        }
        Ok(())
    }

    /// Accept an import only after deterministic chapters and its durable job
    /// have committed atomically. Provider enrichment remains asynchronous.
    #[tracing::instrument(
        skip(self, cmd),
        fields(user_id = %cmd.user_id, title = %cmd.title)
    )]
    pub async fn handle_import(self: &Arc<Self>, cmd: ImportNovelCommand) -> Result<Uuid> {
        info!("Importing novel: {}", cmd.title);

        let raw_text = cmd
            .raw_content
            .ok_or_else(|| anyhow::anyhow!("Novel content is required"))?;
        let admission = self.try_admit_import(cmd.user_id)?;

        let mut novel = Novel::create(cmd.user_id, cmd.title.clone(), cmd.author.clone());
        if let Some(mode) = cmd.deviation_mode {
            novel.set_deviation_mode(mode);
        }
        let novel_id = novel.id;
        // With source retention, acceptance commits at the `source` stage and
        // the claimed worker rebuilds deterministic chapters from the retained
        // object; the request-time extraction above only validates the input.
        let source_stage = cmd.source_bytes.is_some() && self.source_storage.is_some();
        let (chapters, admission) = if source_stage {
            (Vec::new(), admission)
        } else {
            tokio::task::spawn_blocking(move || {
                let chapters = NovelParserService::parse_chapters(novel_id, &raw_text)?;
                ensure_import_budget(&chapters)?;
                Ok::<_, anyhow::Error>((chapters, admission))
            })
            .await
            .map_err(|error| anyhow::anyhow!("chapter parser task failed: {error}"))??
        };
        retain_source_file(
            &mut novel,
            cmd.source_bytes,
            self.source_storage.as_deref(),
            self.source_deletions.as_ref(),
        )
        .await?;
        if source_stage {
            self.novel_repo.create_source_import(&novel).await?;
        } else {
            self.novel_repo.create_import(&novel, &chapters).await?;
        }
        match self.novel_repo.claim_import(novel_id, cmd.user_id).await {
            Ok(Some(claim)) => self.spawn_claimed_import(claim, admission),
            Ok(None) => {
                info!(%novel_id, "durable novel import was claimed by another worker");
            }
            Err(error) => {
                error!(error = ?error, %novel_id, "durable novel import awaits recovery");
            }
        }

        Ok(novel_id)
    }

    /// Retry enrichment for an owned failed import using the chapters that were
    /// already persisted. The original upload does not need to be sent again.
    pub async fn retry_import(self: &Arc<Self>, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        let novel = self
            .novel_repo
            .find_by_id(novel_id)
            .await?
            .filter(|novel| novel.user_id == user_id)
            .ok_or(ImportRetryConflict("Novel cannot be retried"))?;
        if !matches!(novel.status, NovelStatus::Error) {
            return Err(ImportRetryConflict("Only failed imports can be retried").into());
        }

        let chapters = self.chapter_repo.find_by_novel(novel_id).await?;
        if chapters_are_importable(&chapters) {
            ensure_import_budget(&chapters)?;
        } else if novel.file_key.is_none() || self.source_storage.is_none() {
            return Err(ImportRetryConflict(
                "No parsed chapters are available; re-upload the source",
            )
            .into());
        }
        // A stage-`source` job with a retained key replays the object; the
        // import budget is enforced again after the replayed split.

        let admission = self.try_admit_import(user_id)?;
        let claim = match self.novel_repo.claim_import(novel_id, user_id).await? {
            Some(claim) => claim,
            None => {
                // A claim at the import-provider budget ceiling terminates the
                // job; surface its guidance without any provider call.
                let novel = self.novel_repo.find_by_id(novel_id).await?;
                if novel
                    .as_ref()
                    .and_then(|novel| novel.parse_error.as_deref())
                    == Some(IMPORT_BUDGET_EXHAUSTED_MESSAGE)
                {
                    return Err(ImportRetryConflict(IMPORT_BUDGET_EXHAUSTED_MESSAGE).into());
                }
                return Err(ImportRetryConflict("Import cannot be retried").into());
            }
        };
        self.spawn_claimed_import(claim, admission);
        Ok(())
    }

    fn spawn_claimed_import(self: &Arc<Self>, claim: ImportClaim, admission: ImportAdmission) {
        let handler = self.clone();
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                handler.run_claimed_import(claim, admission).await;
            }
            .instrument(current_span),
        );
    }

    async fn run_claimed_import(&self, claim: ImportClaim, admission: ImportAdmission) {
        let _admission = admission;
        let mut lease = ImportLease::start(self.novel_repo.clone(), claim.novel_id, claim.attempt);
        match lease.run(self.process_import(&claim)).await {
            None => tracing::warn!(
                novel_id = %claim.novel_id,
                attempt = claim.attempt,
                "novel import stopped after losing its lease"
            ),
            Some(Err(error)) if error.downcast_ref::<ImportLeaseLost>().is_some() => {
                tracing::warn!(
                    novel_id = %claim.novel_id,
                    attempt = claim.attempt,
                    "novel import write was fenced"
                );
            }
            Some(Err(error)) => {
                lease.stop();
                error!(
                    error = ?error,
                    novel_id = %claim.novel_id,
                    attempt = claim.attempt,
                    "novel import processing failed"
                );
                let (code, public_error) = if error.downcast_ref::<ImportSourceMissing>().is_some()
                {
                    (
                        "source_missing",
                        "The retained source file is missing; re-upload the source",
                    )
                } else if error
                    .downcast_ref::<SourceFileStorageUnavailable>()
                    .is_some()
                {
                    (
                        "source_storage_unavailable",
                        "Source storage is unavailable; retry the import",
                    )
                } else if error.downcast_ref::<ImportChaptersUnusable>().is_some() {
                    (
                        "source_unavailable",
                        "No parsed chapters are available; re-upload the source",
                    )
                } else if claim.stage == ImportStage::Source {
                    (
                        "source_invalid",
                        "The retained source file cannot be parsed; re-upload the source",
                    )
                } else {
                    (
                        "processing_failed",
                        "Import processing failed; retry the import",
                    )
                };
                if let Err(failure_error) = self
                    .novel_repo
                    .fail_import(claim.novel_id, claim.attempt, code, public_error)
                    .await
                {
                    error!(
                        error = ?failure_error,
                        novel_id = %claim.novel_id,
                        attempt = claim.attempt,
                        "novel import failure could not be recorded"
                    );
                }
            }
            Some(Ok(characters)) => {
                lease.stop();
                self.generate_avatars(claim.novel_id, &characters).await;
            }
        }
    }

    async fn process_import(&self, claim: &ImportClaim) -> Result<Vec<Character>> {
        let mut novel = self
            .novel_repo
            .find_by_id(claim.novel_id)
            .await?
            .filter(|novel| novel.user_id == claim.user_id)
            .ok_or_else(|| anyhow::anyhow!("Novel not found"))?;
        let chapters = match claim.stage {
            ImportStage::Source => self.replay_source_chapters(&novel, claim).await?,
            _ => {
                let chapters = self.chapter_repo.find_by_novel(claim.novel_id).await?;
                if !chapters_are_importable(&chapters) {
                    return Err(ImportChaptersUnusable.into());
                }
                chapters
            }
        };
        ensure_import_budget(&chapters)?;

        let characters = match claim.stage {
            ImportStage::Chapters | ImportStage::Source => {
                self.enrich_novel_async(&mut novel, &chapters, claim)
                    .await?
            }
            ImportStage::Enriched => {
                let characters = self.character_repo.find_by_novel(claim.novel_id).await?;
                if characters.is_empty() {
                    return Err(anyhow::anyhow!("enriched import has no characters"));
                }
                characters
            }
            ImportStage::Completed => return Err(ImportLeaseLost.into()),
        };
        self.complete_canon_async(&novel, &chapters, &characters, claim)
            .await?;
        Ok(characters)
    }

    /// Rebuild deterministic chapters from the retained source object and
    /// fenced-commit them, advancing the durable stage from `source` to
    /// `chapters` before any provider call.
    async fn replay_source_chapters(
        &self,
        novel: &Novel,
        claim: &ImportClaim,
    ) -> Result<Vec<Chapter>> {
        let key = novel.file_key.clone().ok_or(ImportSourceMissing)?;
        let storage = self.source_storage.as_ref().ok_or(ImportSourceMissing)?;
        let Some(bytes) = storage
            .get(&key)
            .await
            .map_err(SourceFileStorageUnavailable)?
        else {
            return Err(ImportSourceMissing.into());
        };
        let object_bytes = bytes.len();
        let extractor = self.document_extractor.clone();
        let novel_id = novel.id;
        let chapters = tokio::task::spawn_blocking(move || {
            // The bytes were validated at upload under a server-generated,
            // single-writer key, so magic sniffing is sound for replay: ZIP
            // containers are EPUB, `%PDF-` is PDF, everything else is the
            // stored text upload. This never widens the accepted envelope.
            let mime = if bytes.starts_with(b"PK\x03\x04") {
                "application/epub+zip"
            } else if bytes.starts_with(b"%PDF-") {
                "application/pdf"
            } else {
                "text/plain"
            };
            let text = extractor.extract_text(None, Some(mime), &bytes)?;
            let chapters = NovelParserService::parse_chapters(novel_id, &text)?;
            ensure_import_budget(&chapters)?;
            Ok::<_, anyhow::Error>(chapters)
        })
        .await
        .map_err(|error| anyhow::anyhow!("chapter parser task failed: {error}"))??;
        if !self
            .novel_repo
            .replace_import_chapters(claim.novel_id, claim.attempt, &chapters)
            .await?
        {
            return Err(ImportLeaseLost.into());
        }
        info!(
            novel_id = %claim.novel_id,
            key = %key,
            bytes = object_bytes,
            chapters = chapters.len(),
            "novel import replayed retained source chapters"
        );
        Ok(chapters)
    }

    #[tracing::instrument(skip_all, fields(novel_id = %novel.id))]
    async fn enrich_novel_async(
        &self,
        novel: &mut Novel,
        chapters: &[Chapter],
        claim: &ImportClaim,
    ) -> Result<Vec<Character>> {
        let novel_id = novel.id;
        let total_chapters = chapters.len() as i32;
        let title = novel.title.clone();

        // 提取角色和世界观（代表性样本 + 分块全文扫描）
        info!("Extracting characters for novel {}", novel_id);
        let sample_text = build_representative_sample(chapters);
        let prompt = build_extraction_prompt(&title, &sample_text);
        let extraction_json = self
            .llm
            .chat_json(NovelLlmTask::CharacterExtraction, &prompt)
            .await?;
        let base_extraction: ExtractionResult =
            serde_json::from_str(json_object_payload(&extraction_json))?;
        validate_extraction(&base_extraction)?;

        let mut chunk_extractions = Vec::new();
        if needs_chunk_scan(chapters) {
            let scans = build_scan_plan(chapters);
            let results = stream::iter(scans.into_iter().enumerate())
                .map(|(index, chunk)| {
                    let llm = self.llm.clone();
                    let title = title.clone();
                    async move {
                        let prompt = build_chunk_extraction_prompt(&title, &chunk, index);
                        let json = llm
                            .chat_json(NovelLlmTask::CharacterExtraction, &prompt)
                            .await?;
                        let result: ChunkExtractionResult =
                            serde_json::from_str(json_object_payload(&json))?;
                        validate_chunk_extraction(&result)?;
                        Ok::<_, anyhow::Error>((index, result))
                    }
                })
                .buffer_unordered(3)
                .collect::<Vec<_>>()
                .await;
            for result in results {
                match result {
                    Ok((_, extraction)) => chunk_extractions.push(extraction),
                    Err(error) => tracing::warn!(
                        novel_id = %novel_id,
                        %error,
                        "optional full-text character scan failed; keeping successful scans"
                    ),
                }
            }
        }
        let extraction = merge_extractions(base_extraction, chunk_extractions);
        validate_extraction(&extraction)?;

        // 保存角色
        let characters: Vec<Character> = extraction
            .characters
            .iter()
            .filter_map(|ec| {
                let first_appearance = find_first_appearance(ec, chapters);
                let Some(first_appearance) = first_appearance else {
                    tracing::warn!(character = %ec.name, "omitting character without a source-proven first appearance");
                    return None;
                };
                let Some(mut character) =
                    Character::from_extraction(novel_id, ec, &extraction.world_summary, &title)
                else {
                    tracing::warn!(character = %ec.name, "omitting character with invalid name");
                    return None;
                };
                character.first_appearance_chapter = Some(first_appearance);
                Some(character)
            })
            .collect();

        if characters.is_empty() {
            return Err(anyhow::anyhow!(
                "No extracted character has a verifiable first appearance"
            ));
        }

        let char_name_to_id: HashMap<String, Uuid> = characters
            .iter()
            .map(|character| (character.name.to_lowercase(), character.id))
            .collect();
        let relationships = extraction
            .relationships
            .iter()
            .filter_map(|relationship| {
                let from_id =
                    char_name_to_id.get(&relationship.from_character.trim().to_lowercase());
                let to_id = char_name_to_id.get(&relationship.to_character.trim().to_lowercase());
                Some(CharacterRelationshipRecord {
                    id: Uuid::new_v4(),
                    novel_id,
                    from_character_id: *from_id?,
                    to_character_id: *to_id?,
                    relationship_type: relationship.relationship_type.clone(),
                    description: Some(relationship.description.clone()),
                    strength: relationship.strength,
                })
            })
            .collect::<Vec<_>>();
        if !self
            .character_repo
            .replace_import(novel_id, claim.attempt, &characters, &relationships)
            .await?
        {
            return Err(ImportLeaseLost.into());
        }

        // Detect narrative branch nodes
        info!("Detecting narrative nodes for novel {}", novel_id);
        let chapter_summaries: Vec<(i32, &str)> = chapters
            .iter()
            .map(|c| (c.chapter_number, c.content.as_str()))
            .collect();
        let node_prompt = node_detector::build_node_detection_prompt(&title, &chapter_summaries);
        match self
            .llm
            .chat_json(NovelLlmTask::NarrativeNodeDetection, &node_prompt)
            .await
        {
            Ok(node_json) => {
                match serde_json::from_str::<node_detector::NodeDetectionResult>(
                    json_object_payload(&node_json),
                ) {
                    Ok(detection) => {
                        let validation = node_detector::validate_detection(
                            &detection,
                            chapters.iter().map(|chapter| chapter.chapter_number),
                        );
                        if let Err(error) = validation {
                            tracing::warn!(%error, %novel_id, "node detection output rejected");
                        } else {
                            let nodes = detection
                                .nodes
                                .iter()
                                .map(|node| (node.chapter_number, node.description.clone()))
                                .collect::<Vec<_>>();
                            if !self
                                .chapter_repo
                                .replace_import_nodes(novel_id, claim.attempt, &nodes)
                                .await?
                            {
                                return Err(ImportLeaseLost.into());
                            }
                            info!(
                                "Detected {} narrative nodes for novel {}",
                                detection.nodes.len(),
                                novel_id
                            );
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Node detection JSON parse failed for {}: {}", novel_id, e)
                    }
                }
            }
            Err(e) => tracing::warn!("Node detection LLM call failed for {}: {}", novel_id, e),
        }

        if !self
            .novel_repo
            .record_import_enrichment(
                novel_id,
                claim.attempt,
                total_chapters,
                &extraction.world_summary,
                &extraction.genre,
            )
            .await?
        {
            return Err(ImportLeaseLost.into());
        }
        novel.record_enrichment(
            total_chapters,
            extraction.world_summary.clone(),
            extraction.genre.clone(),
        );
        Ok(characters)
    }

    async fn complete_canon_async(
        &self,
        novel: &Novel,
        chapters: &[Chapter],
        characters: &[Character],
        claim: &ImportClaim,
    ) -> Result<()> {
        if self.canon_repo.find_latest(novel.id).await?.is_none() {
            info!(novel_id = %novel.id, "Extracting canonical story model");
            let chunks = canon_story_extractor::build_scan_plan(chapters)?;
            let results = stream::iter(chunks.into_iter().enumerate())
                .map(|(position, chunk)| {
                    let llm = self.llm.clone();
                    let title = novel.title.clone();
                    async move {
                        let prompt =
                            canon_story_extractor::build_prompt(&title, &chunk, characters)?;
                        // Live-provider robustness: the extraction LLM is
                        // stochastic and can produce evidence excerpts that
                        // violate the strict source-verbatim gate (contiguity,
                        // truncation, punctuation normalization). The mock is
                        // deterministic (never exercises this), so the strict
                        // gate stays intact; a bounded retry lets a valid
                        // stochastic pass land instead of failing a valid source.
                        let mut last_error = None;
                        let mut extraction = None;
                        for attempt in 0..3 {
                            let raw = llm
                                .chat_json(NovelLlmTask::CanonExtraction, &prompt)
                                .await?;
                            match canon_story_extractor::parse_chunk(&raw, &chunk) {
                                Ok(parsed) => {
                                    extraction = Some(parsed);
                                    break;
                                }
                                Err(error) => {
                                    if attempt < 2 {
                                        tracing::debug!(
                                            %error, attempt,
                                            "canonical extraction failed the verbatim gate; retrying"
                                        );
                                    }
                                    last_error = Some(error);
                                }
                            }
                        }
                        let extraction = extraction.ok_or_else(|| {
                            anyhow::anyhow!(
                                "canonical extraction failed the verbatim gate after 3 attempts: {:?}",
                                last_error
                            )
                        })?;
                        Ok::<_, anyhow::Error>((position, chunk, extraction))
                    }
                })
                .buffer_unordered(3)
                .collect::<Vec<_>>()
                .await;
            let mut extracted = results.into_iter().collect::<Result<Vec<_>>>()?;
            extracted.sort_by_key(|(position, _, _)| *position);
            let extracted = extracted
                .into_iter()
                .map(|(_, chunk, extraction)| (chunk, extraction))
                .collect::<Vec<_>>();
            let model = canon_story_extractor::assemble_model(novel.id, 1, &extracted, characters)?;
            if !self.canon_repo.insert_import(&model, claim.attempt).await? {
                return Err(ImportLeaseLost.into());
            }
        }
        if !self
            .novel_repo
            .complete_import(novel.id, claim.attempt)
            .await?
        {
            return Err(ImportLeaseLost.into());
        }
        info!(
            novel_id = %novel.id,
            chapters = chapters.len(),
            characters = characters.len(),
            "novel import completed"
        );
        Ok(())
    }

    async fn generate_avatars(&self, novel_id: Uuid, characters: &[Character]) {
        // ponytail: discover every character; cap cosmetic avatar cost until demand proves otherwise.
        if characters.len() > MAX_AVATARS_PER_NOVEL {
            info!(
                novel_id = %novel_id,
                skipped = characters.len() - MAX_AVATARS_PER_NOVEL,
                "avatar generation capped; all characters remain available"
            );
        }
        let avatar_jobs = characters
            .iter()
            .take(MAX_AVATARS_PER_NOVEL)
            .filter_map(|character| {
                character
                    .appearance
                    .clone()
                    .map(|appearance| (character.id, appearance))
            });
        stream::iter(avatar_jobs)
            .for_each_concurrent(3, |(character_id, appearance)| {
                let character_repo = self.character_repo.clone();
                let image_client = self.image_client.clone();
                async move {
                    if let Err(error) = Self::generate_avatar(
                        character_id,
                        &appearance,
                        character_repo,
                        image_client,
                    )
                    .await
                    {
                        error!(%error, %character_id, "avatar generation failed");
                    }
                }
            })
            .await;
    }

    async fn generate_avatar(
        character_id: Uuid,
        appearance: &str,
        character_repo: Arc<dyn CharacterRepository>,
        image_client: Arc<dyn ImagePort>,
    ) -> Result<()> {
        let appearance: String = appearance.chars().take(2_000).collect();
        let prompt = format!(
            "Portrait of a fictional character. {appearance}. \
            Anime/illustration style, high quality, detailed face, \
            dramatic lighting, cosmic background with stars.",
            appearance = appearance
        );
        let url = image_client.generate(&prompt).await?;
        character_repo.set_avatar(character_id, &url).await?;
        Ok(())
    }
}

fn source_file_key(user_id: Uuid, novel_id: Uuid) -> String {
    format!("source-files/{user_id}/{novel_id}")
}

async fn retain_source_file(
    novel: &mut Novel,
    source_bytes: Option<bytes::Bytes>,
    storage: Option<&dyn SourceFileStorage>,
    deletions: &dyn SourceFileDeletionRepository,
) -> Result<()> {
    let (Some(data), Some(storage)) = (source_bytes, storage) else {
        return Ok(());
    };
    let key = source_file_key(novel.user_id, novel.id);
    deletions
        .enqueue(&key, Utc::now() + ChronoDuration::minutes(5))
        .await?;
    storage
        .put(&key, data)
        .await
        .map_err(SourceFileStorageUnavailable)?;
    novel.retain_source_file(key);
    Ok(())
}

#[cfg(test)]
mod import_budget_tests {
    use super::*;

    struct RecordingStorage {
        events: Arc<Mutex<Vec<&'static str>>>,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl SourceFileStorage for RecordingStorage {
        async fn put(&self, _key: &str, _data: bytes::Bytes) -> Result<()> {
            self.events.lock().unwrap().push("put");
            if self.fail {
                anyhow::bail!("simulated S3 failure");
            }
            Ok(())
        }

        async fn get(&self, _key: &str) -> Result<Option<bytes::Bytes>> {
            self.events.lock().unwrap().push("get");
            Ok(None)
        }

        async fn delete(&self, _key: &str) -> Result<()> {
            Ok(())
        }
    }

    struct RecordingDeletions(Arc<Mutex<Vec<&'static str>>>);

    #[async_trait::async_trait]
    impl SourceFileDeletionRepository for RecordingDeletions {
        async fn enqueue(
            &self,
            _object_key: &str,
            _not_before: chrono::DateTime<Utc>,
        ) -> Result<()> {
            self.0.lock().unwrap().push("enqueue");
            Ok(())
        }

        async fn due(
            &self,
            _limit: i64,
        ) -> Result<Vec<crate::domain::repositories::PendingSourceFileDeletion>> {
            Ok(vec![])
        }

        async fn complete(&self, _object_key: &str) -> Result<()> {
            Ok(())
        }

        async fn retry(
            &self,
            _object_key: &str,
            _error: &str,
            _not_before: chrono::DateTime<Utc>,
        ) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn import_budget_accepts_paste_limit_and_rejects_provider_fanout() {
        let novel_id = Uuid::new_v4();
        let at_paste_limit = vec![Chapter::new(novel_id, 1, None, "a".repeat(5 * 1024 * 1024))];
        let above_budget = vec![Chapter::new(novel_id, 1, None, "a".repeat(6 * 1024 * 1024))];

        assert!(ensure_import_budget(&at_paste_limit).is_ok());
        assert!(ensure_import_budget(&above_budget).is_err());
    }

    #[test]
    fn import_admission_is_per_user_and_service_wide() {
        let permits = Arc::new(Semaphore::new(2));
        let users = Arc::new(Mutex::new(HashSet::new()));
        let first_user = Uuid::new_v4();
        let second_user = Uuid::new_v4();
        let first = try_import_admission(&permits, &users, first_user).unwrap();
        assert!(try_import_admission(&permits, &users, first_user).is_err());
        let second = try_import_admission(&permits, &users, second_user).unwrap();
        assert!(try_import_admission(&permits, &users, Uuid::new_v4()).is_err());
        drop(first);
        drop(second);
        assert!(try_import_admission(&permits, &users, first_user).is_ok());
    }

    #[test]
    fn source_file_key_ignores_untrusted_file_names() {
        let user_id = Uuid::nil();
        let novel_id = Uuid::from_u128(1);
        assert_eq!(
            source_file_key(user_id, novel_id),
            "source-files/00000000-0000-0000-0000-000000000000/00000000-0000-0000-0000-000000000001"
        );
    }

    #[tokio::test]
    async fn source_retention_queues_cleanup_before_write_and_binds_only_on_success() {
        let events = Arc::new(Mutex::new(vec![]));
        let deletions = RecordingDeletions(events.clone());
        let mut novel = Novel::create(Uuid::new_v4(), "test".into(), None);
        retain_source_file(
            &mut novel,
            Some(bytes::Bytes::from_static(b"novel")),
            Some(&RecordingStorage {
                events: events.clone(),
                fail: false,
            }),
            &deletions,
        )
        .await
        .unwrap();
        assert_eq!(*events.lock().unwrap(), ["enqueue", "put"]);
        assert!(novel.file_key.is_some());

        events.lock().unwrap().clear();
        let mut failed_novel = Novel::create(Uuid::new_v4(), "test".into(), None);
        let error = retain_source_file(
            &mut failed_novel,
            Some(bytes::Bytes::from_static(b"novel")),
            Some(&RecordingStorage {
                events: events.clone(),
                fail: true,
            }),
            &deletions,
        )
        .await
        .unwrap_err();
        assert!(error
            .downcast_ref::<SourceFileStorageUnavailable>()
            .is_some());
        assert_eq!(*events.lock().unwrap(), ["enqueue", "put"]);
        assert!(failed_novel.file_key.is_none());
    }
}

const MAX_READER_IDENTITY_CHARS: usize = 200;
const MAX_LORE_QUERY_CHARS: usize = 1_000;
const MAX_LORE_RESULTS: usize = 5;

#[derive(Debug, thiserror::Error)]
pub enum ReadingProgressError {
    #[error("Novel not found")]
    NotFound,
    #[error("Character not found")]
    CharacterNotFound,
    #[error("{0}")]
    Validation(String),
    #[error("Reading progress operation failed")]
    Internal(#[source] anyhow::Error),
}

fn validate_chapter_number(
    chapter: i32,
    total_chapters: i32,
) -> std::result::Result<(), ReadingProgressError> {
    if chapter < 1 || chapter > total_chapters {
        return Err(ReadingProgressError::Validation(format!(
            "chapter must be between 1 and {total_chapters}"
        )));
    }
    Ok(())
}

fn normalize_identity_name(
    identity_name: Option<String>,
) -> std::result::Result<Option<String>, ReadingProgressError> {
    if identity_name
        .as_deref()
        .is_some_and(|name| name.chars().any(char::is_control))
    {
        return Err(ReadingProgressError::Validation(
            "identity_name must not contain control characters".into(),
        ));
    }
    let identity_name = identity_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty());
    if identity_name
        .as_deref()
        .is_some_and(|name| name.chars().count() > MAX_READER_IDENTITY_CHARS)
    {
        return Err(ReadingProgressError::Validation(format!(
            "identity_name must not exceed {MAX_READER_IDENTITY_CHARS} characters"
        )));
    }
    Ok(identity_name)
}

fn character_name_is_canonical(name: &str) -> bool {
    normalize_identity_name(Some(name.to_owned()))
        .is_ok_and(|normalized| normalized.as_deref() == Some(name))
}

fn validate_character_appearance(
    first_appearance_chapter: Option<i32>,
    current_chapter: i32,
) -> std::result::Result<(), ReadingProgressError> {
    if !first_appearance_chapter.is_some_and(|chapter| (1..=current_chapter).contains(&chapter)) {
        return Err(ReadingProgressError::Validation(
            "reader character appearance is unknown or later than the current chapter".into(),
        ));
    }
    Ok(())
}

fn normalize_lore_query(query: &str) -> std::result::Result<String, ReadingProgressError> {
    let query = query.split_whitespace().collect::<Vec<_>>().join(" ");
    if query.is_empty() {
        return Err(ReadingProgressError::Validation(
            "lore query must not be empty".into(),
        ));
    }
    if query.chars().count() > MAX_LORE_QUERY_CHARS {
        return Err(ReadingProgressError::Validation(format!(
            "lore query must not exceed {MAX_LORE_QUERY_CHARS} characters"
        )));
    }
    Ok(query)
}

fn lore_chapter_limit(
    requested: i32,
    current: i32,
) -> std::result::Result<i32, ReadingProgressError> {
    if requested < 1 {
        return Err(ReadingProgressError::Validation(
            "max_chapter must be at least 1".into(),
        ));
    }
    Ok(requested.min(current))
}

pub struct ReadingProgressHandler {
    pub novel_repo: Arc<dyn NovelRepository>,
    pub chapter_repo: Arc<dyn ChapterRepository>,
    pub character_repo: Arc<dyn CharacterRepository>,
    pub progress_repo: Arc<dyn ReadingProgressRepository>,
}

impl ReadingProgressHandler {
    async fn owned_novel(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> std::result::Result<crate::domain::entities::novel::Novel, ReadingProgressError> {
        match self
            .novel_repo
            .find_by_id(novel_id)
            .await
            .map_err(ReadingProgressError::Internal)?
        {
            Some(novel) if novel.user_id == user_id => Ok(novel),
            _ => Err(ReadingProgressError::NotFound),
        }
    }

    async fn progress_for_novel(
        &self,
        user_id: Uuid,
        novel: &Novel,
    ) -> std::result::Result<ReadingProgressRecord, ReadingProgressError> {
        if novel.total_chapters < 1 {
            return Err(ReadingProgressError::Validation(
                "novel has no readable chapters".into(),
            ));
        }
        let progress = self
            .progress_repo
            .get_or_create(user_id, novel.id, novel.deviation_mode.to_str())
            .await
            .map_err(ReadingProgressError::Internal)?;
        validate_chapter_number(progress.current_chapter, novel.total_chapters).map_err(|_| {
            ReadingProgressError::Internal(anyhow::anyhow!(
                "persisted reading progress points outside the novel"
            ))
        })?;
        if self
            .chapter_repo
            .find_by_number(novel.id, progress.current_chapter)
            .await
            .map_err(ReadingProgressError::Internal)?
            .is_none()
        {
            return Err(ReadingProgressError::Internal(anyhow::anyhow!(
                "persisted reading progress points to a missing chapter"
            )));
        }
        Ok(progress)
    }

    pub async fn get(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> std::result::Result<ReadingProgressRecord, ReadingProgressError> {
        let novel = self.owned_novel(user_id, novel_id).await?;
        let progress = self.progress_for_novel(user_id, &novel).await?;

        let persisted_identity = normalize_identity_name(progress.reader_identity.clone())
            .map_err(|_| {
                ReadingProgressError::Internal(anyhow::anyhow!(
                    "persisted reader identity is invalid"
                ))
            })?;
        if persisted_identity != progress.reader_identity {
            return Err(ReadingProgressError::Internal(anyhow::anyhow!(
                "persisted reader identity is not normalized"
            )));
        }
        match ReaderIdentityType::from_str(&progress.reader_identity_type) {
            Some(ReaderIdentityType::Self_) if progress.reader_character_id.is_none() => {}
            Some(ReaderIdentityType::Character) => {
                let character_id = progress.reader_character_id.ok_or_else(|| {
                    ReadingProgressError::Internal(anyhow::anyhow!(
                        "persisted character identity has no character"
                    ))
                })?;
                let character = self
                    .character_repo
                    .find_by_id(character_id)
                    .await
                    .map_err(ReadingProgressError::Internal)?
                    .filter(|character| character.novel_id == novel_id)
                    .ok_or_else(|| {
                        ReadingProgressError::Internal(anyhow::anyhow!(
                            "persisted reader character does not belong to the novel"
                        ))
                    })?;
                validate_character_appearance(
                    character.first_appearance_chapter,
                    progress.current_chapter,
                )
                .map_err(|_| {
                    ReadingProgressError::Internal(anyhow::anyhow!(
                        "persisted reader character has not appeared yet"
                    ))
                })?;
                if progress.reader_identity.as_deref() != Some(character.name.as_str()) {
                    return Err(ReadingProgressError::Internal(anyhow::anyhow!(
                        "persisted reader character name is invalid"
                    )));
                }
            }
            _ => {
                return Err(ReadingProgressError::Internal(anyhow::anyhow!(
                    "persisted reader identity fields are inconsistent"
                )));
            }
        }
        Ok(progress)
    }

    pub async fn update_chapter(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter: i32,
    ) -> std::result::Result<(), ReadingProgressError> {
        let novel = self.owned_novel(user_id, novel_id).await?;
        self.progress_for_novel(user_id, &novel).await?;
        validate_chapter_number(chapter, novel.total_chapters)?;
        if self
            .chapter_repo
            .find_by_number(novel_id, chapter)
            .await
            .map_err(ReadingProgressError::Internal)?
            .is_none()
        {
            return Err(ReadingProgressError::Validation(
                "chapter does not exist".into(),
            ));
        }

        self.progress_repo
            .update_chapter(user_id, novel_id, chapter)
            .await
            .map_err(ReadingProgressError::Internal)
    }

    pub async fn search_lore(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        requested_max_chapter: i32,
        query: &str,
        limit: usize,
    ) -> std::result::Result<Vec<LoreExcerpt>, ReadingProgressError> {
        let query = normalize_lore_query(query)?;
        let novel = self.owned_novel(user_id, novel_id).await?;
        let progress = self.progress_for_novel(user_id, &novel).await?;
        let max_chapter = lore_chapter_limit(requested_max_chapter, progress.current_chapter)?;

        self.chapter_repo
            .search_lore(
                novel_id,
                max_chapter,
                &query,
                limit.clamp(1, MAX_LORE_RESULTS),
            )
            .await
            .map_err(ReadingProgressError::Internal)
    }

    pub async fn list_available_characters(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> std::result::Result<Vec<Character>, ReadingProgressError> {
        let novel = self.owned_novel(user_id, novel_id).await?;
        let progress = self.progress_for_novel(user_id, &novel).await?;
        let characters = self
            .character_repo
            .find_by_novel(novel_id)
            .await
            .map_err(ReadingProgressError::Internal)?;
        Ok(characters
            .into_iter()
            .filter(|character| {
                character_name_is_canonical(&character.name)
                    && validate_character_appearance(
                        character.first_appearance_chapter,
                        progress.current_chapter,
                    )
                    .is_ok()
            })
            .collect())
    }

    pub async fn get_available_character(
        &self,
        user_id: Uuid,
        character_id: Uuid,
    ) -> std::result::Result<Character, ReadingProgressError> {
        let character = self
            .character_repo
            .find_by_id(character_id)
            .await
            .map_err(ReadingProgressError::Internal)?
            .ok_or(ReadingProgressError::CharacterNotFound)?;
        let novel = self.owned_novel(user_id, character.novel_id).await?;
        let progress = self.progress_for_novel(user_id, &novel).await?;
        validate_character_appearance(character.first_appearance_chapter, progress.current_chapter)
            .map_err(|_| ReadingProgressError::CharacterNotFound)?;
        if !character_name_is_canonical(&character.name) {
            return Err(ReadingProgressError::CharacterNotFound);
        }
        Ok(character)
    }

    pub async fn set_identity(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        identity_type: &str,
        identity_name: Option<String>,
        character_id: Option<Uuid>,
    ) -> std::result::Result<(), ReadingProgressError> {
        let novel = self.owned_novel(user_id, novel_id).await?;
        let progress = self.progress_for_novel(user_id, &novel).await?;
        let identity_type = ReaderIdentityType::from_str(identity_type).ok_or_else(|| {
            ReadingProgressError::Validation(
                "identity_type must be either self or character".into(),
            )
        })?;

        let identity_name = normalize_identity_name(identity_name)?;

        let (identity_name, character_id) = match identity_type {
            ReaderIdentityType::Self_ => {
                if character_id.is_some() {
                    return Err(ReadingProgressError::Validation(
                        "self identity must not include character_id".into(),
                    ));
                }
                (identity_name, None)
            }
            ReaderIdentityType::Character => {
                let character_id = character_id.ok_or_else(|| {
                    ReadingProgressError::Validation(
                        "character identity requires character_id".into(),
                    )
                })?;
                let character = self
                    .character_repo
                    .find_by_id(character_id)
                    .await
                    .map_err(ReadingProgressError::Internal)?
                    .filter(|character| character.novel_id == novel_id)
                    .ok_or_else(|| {
                        ReadingProgressError::Validation(
                            "reader character must belong to the novel".into(),
                        )
                    })?;
                validate_character_appearance(
                    character.first_appearance_chapter,
                    progress.current_chapter,
                )?;
                if !character_name_is_canonical(&character.name) {
                    return Err(ReadingProgressError::Validation(
                        "reader character name is invalid".into(),
                    ));
                }
                (
                    normalize_identity_name(Some(character.name))?,
                    Some(character_id),
                )
            }
        };

        self.progress_repo
            .set_identity(
                user_id,
                novel_id,
                identity_type.to_str(),
                identity_name.as_deref(),
                character_id,
            )
            .await
            .map_err(ReadingProgressError::Internal)
    }
}

#[cfg(test)]
mod reading_progress_validation_tests {
    use super::*;

    #[test]
    fn chapter_must_exist_inside_the_novel_range() {
        assert!(validate_chapter_number(1, 10).is_ok());
        assert!(validate_chapter_number(10, 10).is_ok());
        assert!(validate_chapter_number(0, 10).is_err());
        assert!(validate_chapter_number(-1, 10).is_err());
        assert!(validate_chapter_number(11, 10).is_err());
    }

    #[test]
    fn identity_name_enforces_unicode_and_control_character_bounds() {
        assert_eq!(
            normalize_identity_name(Some("  Reader  ".into())).unwrap(),
            Some("Reader".into())
        );
        assert!(normalize_identity_name(Some("读".repeat(200))).is_ok());
        assert!(normalize_identity_name(Some("读".repeat(201))).is_err());
        assert!(normalize_identity_name(Some("bad\nname".into())).is_err());
        assert!(character_name_is_canonical("Reader"));
        assert!(!character_name_is_canonical("\u{a0}Reader\u{a0}"));
    }

    #[test]
    fn future_character_cannot_become_reader_identity() {
        assert!(validate_character_appearance(Some(3), 2).is_err());
        assert!(validate_character_appearance(Some(2), 2).is_ok());
        assert!(validate_character_appearance(None, 2).is_err());
        assert!(validate_character_appearance(Some(0), 2).is_err());
        assert!(validate_character_appearance(Some(-1), 2).is_err());
    }

    #[test]
    fn lore_queries_are_normalized_and_bounded() {
        assert_eq!(normalize_lore_query("  密室\n蛇怪  ").unwrap(), "密室 蛇怪");
        assert!(normalize_lore_query(" \n ").is_err());
        assert!(normalize_lore_query(&"界".repeat(MAX_LORE_QUERY_CHARS + 1)).is_err());
        assert_eq!(lore_chapter_limit(7, 3).unwrap(), 3);
        assert_eq!(lore_chapter_limit(2, 3).unwrap(), 2);
        assert!(lore_chapter_limit(0, 3).is_err());
    }
}
