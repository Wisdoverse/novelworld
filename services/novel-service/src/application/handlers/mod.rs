use anyhow::Result;
use futures::{stream, StreamExt};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tracing::{error, info};
use uuid::Uuid;

use crate::application::commands::ImportNovelCommand;
use crate::domain::entities::{chapter::Chapter, character::Character, novel::Novel};
use crate::domain::ports::{ImagePort, LlmPort};
use crate::domain::repositories::{
    ChapterRepository, CharacterRepository, LoreExcerpt, NovelRepository, ReadingProgressRecord,
    ReadingProgressRepository,
};
use crate::domain::services::node_detector;
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
    pub llm: Arc<dyn LlmPort>,
    pub image_client: Arc<dyn ImagePort>,
}

const MAX_AVATARS_PER_NOVEL: usize = 30;

impl NovelCommandHandler {
    /// 处理小说导入命令（异步解析流程）
    #[tracing::instrument(
        skip(self, cmd),
        fields(user_id = %cmd.user_id, title = %cmd.title)
    )]
    pub async fn handle_import(&self, cmd: ImportNovelCommand) -> Result<Uuid> {
        info!("Importing novel: {}", cmd.title);

        // 1. 创建 Novel 聚合根
        let mut novel = Novel::create(cmd.user_id, cmd.title.clone(), cmd.author.clone());
        if let Some(mode) = cmd.deviation_mode {
            novel.set_deviation_mode(mode);
        }
        self.novel_repo.save(&novel).await?;

        let novel_id = novel.id;

        // 2. 获取原始文本
        let raw_text = match cmd.raw_content {
            Some(text) => text,
            None => {
                // TODO: 从 S3 下载文件并解析 PDF/TXT
                return Err(anyhow::anyhow!("File upload parsing not yet implemented"));
            }
        };

        // 3. 异步执行解析（不阻塞响应）
        let novel_repo = self.novel_repo.clone();
        let novel_repo_err = self.novel_repo.clone();
        let chapter_repo = self.chapter_repo.clone();
        let character_repo = self.character_repo.clone();
        let llm = self.llm.clone();
        let image_client = self.image_client.clone();
        let title = cmd.title.clone();

        tokio::spawn(async move {
            if let Err(e) = Self::parse_novel_async(
                novel_id,
                &title,
                &raw_text,
                novel_repo,
                chapter_repo,
                character_repo,
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
        if !self
            .character_repo
            .find_by_novel(novel_id)
            .await?
            .is_empty()
        {
            return Err(anyhow::anyhow!(
                "This import has partial character data and cannot be retried safely"
            ));
        }

        novel.start_parsing();
        self.novel_repo.update(&novel).await?;

        let title = novel.title.clone();
        let novel_repo = self.novel_repo.clone();
        let novel_repo_err = self.novel_repo.clone();
        let chapter_repo = self.chapter_repo.clone();
        let character_repo = self.character_repo.clone();
        let llm = self.llm.clone();
        let image_client = self.image_client.clone();
        tokio::spawn(async move {
            if let Err(error) = Self::enrich_novel_async(
                novel,
                &title,
                chapters,
                novel_repo,
                chapter_repo,
                character_repo,
                llm,
                image_client,
            )
            .await
            {
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
        raw_text: &str,
        novel_repo: Arc<dyn NovelRepository>,
        chapter_repo: Arc<dyn ChapterRepository>,
        character_repo: Arc<dyn CharacterRepository>,
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
        let chapters = NovelParserService::parse_chapters(novel_id, raw_text)?;
        chapter_repo.save_batch(&chapters).await?;

        Self::enrich_novel_async(
            novel,
            title,
            chapters,
            novel_repo,
            chapter_repo,
            character_repo,
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
        llm: Arc<dyn LlmPort>,
        image_client: Arc<dyn ImagePort>,
    ) -> Result<()> {
        let novel_id = novel.id;
        let total_chapters = chapters.len() as i32;

        // 提取角色和世界观（代表性样本 + 分块全文扫描）
        info!("Extracting characters for novel {}", novel_id);
        let sample_text = build_representative_sample(&chapters);
        let prompt = build_extraction_prompt(title, &sample_text);
        let extraction_json = llm.chat_json(&prompt).await?;
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
                        let json = llm.chat_json(&prompt).await?;
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
        match llm.chat_json(&node_prompt).await {
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

        // 标记小说为 ready
        novel.mark_ready(
            total_chapters,
            extraction.world_summary.clone(),
            extraction.genre.clone(),
        );
        novel_repo.update(&novel).await?;

        // ponytail: discover every character; cap cosmetic avatar cost until demand proves otherwise.
        let semaphore = Arc::new(Semaphore::new(3));
        if characters.len() > MAX_AVATARS_PER_NOVEL {
            info!(
                novel_id = %novel_id,
                skipped = characters.len() - MAX_AVATARS_PER_NOVEL,
                "avatar generation capped; all characters remain available"
            );
        }
        for character in characters.iter().take(MAX_AVATARS_PER_NOVEL) {
            if let Some(appearance) = &character.appearance {
                let char_id = character.id;
                let appearance = appearance.clone();
                let char_repo = character_repo.clone();
                let img_client = image_client.clone();
                let sem = semaphore.clone();
                tokio::spawn(async move {
                    let _permit = sem.acquire().await.unwrap();
                    if let Err(e) =
                        Self::generate_avatar(char_id, &appearance, char_repo, img_client).await
                    {
                        error!("Avatar generation failed for {}: {}", char_id, e);
                    }
                });
            }
        }

        info!(
            "Novel {} parsed successfully: {} chapters, {} characters",
            novel_id,
            total_chapters,
            characters.len()
        );
        Ok(())
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
