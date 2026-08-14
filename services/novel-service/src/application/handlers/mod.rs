use anyhow::Result;
use chrono::{Duration as ChronoDuration, Utc};
use futures::{stream, StreamExt};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tracing::{error, info};
use uuid::Uuid;

use crate::application::commands::ImportNovelCommand;
use crate::domain::entities::{chapter::Chapter, character::Character, novel::Novel};
use crate::domain::ports::{
    ImagePort, LlmPort, NovelLlmTask, PrivacyCleanupPort, SourceFileStorage,
};
use crate::domain::repositories::{
    CanonStoryModelRepository, ChapterRepository, CharacterRepository, LoreExcerpt,
    NovelRepository, ReadingProgressRecord, ReadingProgressRepository,
    SourceFileDeletionRepository,
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
use crate::domain::value_objects::{NovelStatus, ReaderIdentityType};

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
#[error("Source file storage is unavailable")]
pub struct SourceFileStorageUnavailable(#[source] pub anyhow::Error);

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

    /// 处理小说导入命令（异步解析流程）
    #[tracing::instrument(
        skip(self, cmd),
        fields(user_id = %cmd.user_id, title = %cmd.title)
    )]
    pub async fn handle_import(&self, cmd: ImportNovelCommand) -> Result<Uuid> {
        info!("Importing novel: {}", cmd.title);

        let raw_text = cmd
            .raw_content
            .ok_or_else(|| anyhow::anyhow!("Novel content is required"))?;
        let admission = self.try_admit_import(cmd.user_id)?;

        // 1. 创建 Novel 聚合根
        let mut novel = Novel::create(cmd.user_id, cmd.title.clone(), cmd.author.clone());
        if let Some(mode) = cmd.deviation_mode {
            novel.set_deviation_mode(mode);
        }
        retain_source_file(
            &mut novel,
            cmd.source_bytes,
            self.source_storage.as_deref(),
            self.source_deletions.as_ref(),
        )
        .await?;
        self.novel_repo.save(&novel).await?;

        let novel_id = novel.id;

        // 2. 异步执行解析（不阻塞响应）
        let novel_repo = self.novel_repo.clone();
        let novel_repo_err = self.novel_repo.clone();
        let chapter_repo = self.chapter_repo.clone();
        let character_repo = self.character_repo.clone();
        let canon_repo = self.canon_repo.clone();
        let llm = self.llm.clone();
        let image_client = self.image_client.clone();
        let title = cmd.title.clone();

        tokio::spawn(async move {
            let _admission = admission;
            if let Err(e) = Self::parse_novel_async(
                novel_id,
                &title,
                raw_text,
                novel_repo,
                chapter_repo,
                character_repo,
                canon_repo,
                llm,
                image_client,
            )
            .await
            {
                error!("Novel parsing failed for {}: {}", novel_id, e);
                if let Ok(Some(mut novel)) = novel_repo_err.find_by_id(novel_id).await {
                    novel.mark_error(e.to_string());
                    let _ = novel_repo_err.update(&novel).await;
                }
            }
        });

        Ok(novel_id)
    }

    /// Retry enrichment for an owned failed import using the chapters that were
    /// already persisted. The original upload does not need to be sent again.
    pub async fn retry_import(&self, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        let mut novel = self
            .novel_repo
            .find_by_id(novel_id)
            .await?
            .filter(|novel| novel.user_id == user_id)
            .ok_or_else(|| anyhow::anyhow!("Novel not found"))?;
        if !matches!(novel.status, NovelStatus::Error) {
            return Err(anyhow::anyhow!("Only failed imports can be retried"));
        }

        let chapters = self.chapter_repo.find_by_novel(novel_id).await?;
        if chapters.is_empty() {
            return Err(anyhow::anyhow!(
                "No parsed chapters are available for retry"
            ));
        }
        let chapters = tokio::task::spawn_blocking(move || {
            ensure_import_budget(&chapters)?;
            Ok::<_, anyhow::Error>(chapters)
        })
        .await
        .map_err(|error| anyhow::anyhow!("import budget task failed: {error}"))??;
        let characters = self.character_repo.find_by_novel(novel_id).await?;
        let resume_canon = !characters.is_empty();
        if resume_canon
            && (novel.total_chapters != chapters.len() as i32
                || novel.world_summary.is_none()
                || novel.genre.is_none())
        {
            return Err(anyhow::anyhow!(
                "This legacy partial import cannot be retried safely"
            ));
        }

        let admission = self.try_admit_import(user_id)?;
        novel.start_parsing();
        self.novel_repo.update(&novel).await?;

        let title = novel.title.clone();
        let novel_repo = self.novel_repo.clone();
        let novel_repo_err = self.novel_repo.clone();
        let chapter_repo = self.chapter_repo.clone();
        let character_repo = self.character_repo.clone();
        let canon_repo = self.canon_repo.clone();
        let llm = self.llm.clone();
        let image_client = self.image_client.clone();
        tokio::spawn(async move {
            let _admission = admission;
            let result = if resume_canon {
                Self::complete_canon_async(
                    &mut novel,
                    &chapters,
                    &characters,
                    novel_repo,
                    canon_repo,
                    llm,
                )
                .await
            } else {
                Self::enrich_novel_async(
                    novel,
                    &title,
                    chapters,
                    novel_repo,
                    chapter_repo,
                    character_repo,
                    canon_repo,
                    llm,
                    image_client,
                )
                .await
            };
            if let Err(error) = result {
                error!("Novel retry failed for {}: {}", novel_id, error);
                if let Ok(Some(mut novel)) = novel_repo_err.find_by_id(novel_id).await {
                    novel.mark_error(error.to_string());
                    let _ = novel_repo_err.update(&novel).await;
                }
            }
        });
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(novel_id = %novel_id))]
    // ponytail: keep the one-shot task explicit until ingestion becomes a durable job.
    #[allow(clippy::too_many_arguments)]
    async fn parse_novel_async(
        novel_id: Uuid,
        title: &str,
        raw_text: String,
        novel_repo: Arc<dyn NovelRepository>,
        chapter_repo: Arc<dyn ChapterRepository>,
        character_repo: Arc<dyn CharacterRepository>,
        canon_repo: Arc<dyn CanonStoryModelRepository>,
        llm: Arc<dyn LlmPort>,
        image_client: Arc<dyn ImagePort>,
    ) -> Result<()> {
        // 更新状态为解析中
        let mut novel = novel_repo
            .find_by_id(novel_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Novel not found"))?;
        novel.start_parsing();
        novel_repo.update(&novel).await?;

        // 拆分章节
        info!("Parsing chapters for novel {}", novel_id);
        let chapters = tokio::task::spawn_blocking(move || {
            let chapters = NovelParserService::parse_chapters(novel_id, &raw_text)?;
            ensure_import_budget(&chapters)?;
            Ok::<_, anyhow::Error>(chapters)
        })
        .await
        .map_err(|error| anyhow::anyhow!("chapter parser task failed: {error}"))??;
        chapter_repo.save_batch(&chapters).await?;

        Self::enrich_novel_async(
            novel,
            title,
            chapters,
            novel_repo,
            chapter_repo,
            character_repo,
            canon_repo,
            llm,
            image_client,
        )
        .await
    }

    #[tracing::instrument(skip_all, fields(novel_id = %novel.id))]
    #[allow(clippy::too_many_arguments)]
    async fn enrich_novel_async(
        mut novel: Novel,
        title: &str,
        chapters: Vec<Chapter>,
        novel_repo: Arc<dyn NovelRepository>,
        chapter_repo: Arc<dyn ChapterRepository>,
        character_repo: Arc<dyn CharacterRepository>,
        canon_repo: Arc<dyn CanonStoryModelRepository>,
        llm: Arc<dyn LlmPort>,
        image_client: Arc<dyn ImagePort>,
    ) -> Result<()> {
        let novel_id = novel.id;
        let total_chapters = chapters.len() as i32;

        // 提取角色和世界观（代表性样本 + 分块全文扫描）
        info!("Extracting characters for novel {}", novel_id);
        let sample_text = build_representative_sample(&chapters);
        let prompt = build_extraction_prompt(title, &sample_text);
        let extraction_json = llm
            .chat_json(NovelLlmTask::CharacterExtraction, &prompt)
            .await?;
        let base_extraction: ExtractionResult =
            serde_json::from_str(json_object_payload(&extraction_json))?;
        validate_extraction(&base_extraction)?;

        let mut chunk_extractions = Vec::new();
        if needs_chunk_scan(&chapters) {
            let scans = build_scan_plan(&chapters);
            let results = stream::iter(scans.into_iter().enumerate())
                .map(|(index, chunk)| {
                    let llm = llm.clone();
                    async move {
                        let prompt = build_chunk_extraction_prompt(title, &chunk, index);
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
                let first_appearance = find_first_appearance(ec, &chapters);
                let Some(first_appearance) = first_appearance else {
                    tracing::warn!(character = %ec.name, "omitting character without a source-proven first appearance");
                    return None;
                };
                let Some(mut character) =
                    Character::from_extraction(novel_id, ec, &extraction.world_summary, title)
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

        character_repo.save_batch(&characters).await?;

        // Save character relationship graph
        if !extraction.relationships.is_empty() {
            let char_name_to_id: std::collections::HashMap<String, Uuid> = characters
                .iter()
                .map(|c| (c.name.to_lowercase(), c.id))
                .collect();

            for rel in &extraction.relationships {
                let from_id = char_name_to_id.get(&rel.from_character.trim().to_lowercase());
                let to_id = char_name_to_id.get(&rel.to_character.trim().to_lowercase());
                if let (Some(&from), Some(&to)) = (from_id, to_id) {
                    if let Err(e) = character_repo
                        .save_relationship(
                            novel_id,
                            from,
                            to,
                            &rel.relationship_type,
                            Some(rel.description.as_str()),
                            rel.strength,
                        )
                        .await
                    {
                        tracing::error!(
                            "Failed to save relationship {}->{}: {}",
                            rel.from_character,
                            rel.to_character,
                            e
                        );
                    }
                }
            }
            info!(
                "Saved {} character relationships for novel {}",
                extraction.relationships.len(),
                novel_id
            );
        }

        // Detect narrative branch nodes
        info!("Detecting narrative nodes for novel {}", novel_id);
        let chapter_summaries: Vec<(i32, &str)> = chapters
            .iter()
            .map(|c| (c.chapter_number, c.content.as_str()))
            .collect();
        let node_prompt = node_detector::build_node_detection_prompt(title, &chapter_summaries);
        match llm
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
                            for node in &detection.nodes {
                                if let Some(ch) = chapters
                                    .iter()
                                    .find(|c| c.chapter_number == node.chapter_number)
                                {
                                    // Mark chapter as key node
                                    let mut updated_ch = ch.clone();
                                    updated_ch.mark_as_key_node(node.description.clone());
                                    if let Err(e) = chapter_repo.update(&updated_ch).await {
                                        tracing::error!(
                                            "Failed to mark chapter {} as key node: {}",
                                            node.chapter_number,
                                            e
                                        );
                                    }
                                }
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

        // Persist completed source enrichment before canonical extraction. The
        // novel remains non-ready until the source-complete model commits.
        novel.record_enrichment(
            total_chapters,
            extraction.world_summary.clone(),
            extraction.genre.clone(),
        );
        novel_repo.update(&novel).await?;
        Self::complete_canon_async(
            &mut novel,
            &chapters,
            &characters,
            novel_repo.clone(),
            canon_repo,
            llm,
        )
        .await?;

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
                let character_repo = character_repo.clone();
                let image_client = image_client.clone();
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

        info!(
            "Novel {} parsed successfully: {} chapters, {} characters",
            novel_id,
            total_chapters,
            characters.len()
        );
        Ok(())
    }

    async fn complete_canon_async(
        novel: &mut Novel,
        chapters: &[Chapter],
        characters: &[Character],
        novel_repo: Arc<dyn NovelRepository>,
        canon_repo: Arc<dyn CanonStoryModelRepository>,
        llm: Arc<dyn LlmPort>,
    ) -> Result<()> {
        if canon_repo.find_latest(novel.id).await?.is_none() {
            info!(novel_id = %novel.id, "Extracting canonical story model");
            let chunks = canon_story_extractor::build_scan_plan(chapters)?;
            let results = stream::iter(chunks.into_iter().enumerate())
                .map(|(position, chunk)| {
                    let llm = llm.clone();
                    let title = novel.title.clone();
                    async move {
                        let prompt =
                            canon_story_extractor::build_prompt(&title, &chunk, characters)?;
                        let raw = llm
                            .chat_json(NovelLlmTask::CanonExtraction, &prompt)
                            .await?;
                        let extraction = canon_story_extractor::parse_chunk(&raw, &chunk)?;
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
            canon_repo.insert(&model).await?;
        }

        let total_chapters = novel.total_chapters;
        let world_summary = novel
            .world_summary
            .clone()
            .ok_or_else(|| anyhow::anyhow!("enriched novel has no world summary"))?;
        let genre = novel
            .genre
            .clone()
            .ok_or_else(|| anyhow::anyhow!("enriched novel has no genre"))?;
        novel.mark_ready(total_chapters, world_summary, genre);
        novel_repo.update(novel).await
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
        let mut character = character_repo
            .find_by_id(character_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Character not found"))?;
        character.set_avatar(url);
        character_repo.update(&character).await?;
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
