use anyhow::Result;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use futures::{stream, StreamExt};
use serde::{de::DeserializeOwned, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tokio::sync::{oneshot, watch, OwnedSemaphorePermit, Semaphore};
use tracing::{error, info, warn, Instrument};
use uuid::Uuid;

use crate::application::commands::ImportNovelCommand;
use crate::domain::entities::{
    chapter::{chapters_are_importable, Chapter},
    character::Character,
    game_rule_template::GameRuleTemplate,
    novel::Novel,
};
use crate::domain::ports::{
    DocumentTextExtractor, ImagePort, LlmPort, NovelLlmTask, PrivacyCleanupPort, SourceFileStorage,
    TextTranslator,
};
use crate::domain::repositories::{
    BeginChapterTranslation, BeginGameRuleGeneration, CanonExtractionCheckpoint,
    CanonStoryModelRepository, ChapterRepository, ChapterTranslationKey,
    ChapterTranslationRepository, CharacterRelationshipRecord, CharacterRepository, ImportClaim,
    LoreExcerpt, NovelRepository, ReadingProgressRecord, ReadingProgressRepository,
    SourceFileDeletionRepository, IMPORT_BUDGET_EXHAUSTED_MESSAGE,
};
use crate::domain::services::{
    canon_story_extractor, chapter_boundary_detector, game_rule_generator, node_detector,
};
use crate::domain::services::{
    character_extractor::{
        build_chunk_extraction_prompt, build_extraction_prompt, build_representative_sample,
        build_scan_plan, find_first_appearance, json_object_payload, merge_extractions,
        needs_chunk_scan, text_contains_name, validate_chunk_extraction, validate_extraction,
        ChunkExtractionResult, ExtractionResult,
    },
    novel_parser::NovelParserService,
};
use crate::domain::value_objects::{
    AvatarStatus, CharacterRole, DeviationMode, ImportStage, NovelStatus, ReaderIdentityType,
};

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

pub const MAX_BATCH_IMPORTS: usize = 5;

struct ImportAdmission {
    _permit: OwnedSemaphorePermit,
    active_users: Arc<Mutex<HashSet<Uuid>>>,
    user_id: Uuid,
}

struct PreparedImport {
    novel: Novel,
    chapters: Vec<Chapter>,
    source_bytes: Option<bytes::Bytes>,
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

async fn validated_json<T>(
    llm: &dyn LlmPort,
    user_id: Uuid,
    task: NovelLlmTask,
    prompt: &str,
    validate: impl Fn(&T) -> Result<()>,
) -> Result<T>
where
    T: DeserializeOwned,
{
    let mut last_error = None;
    let mut current_prompt = prompt.to_string();
    for attempt in 1..=3 {
        // Transport retries already live in the shared client. This loop is
        // only for a fresh model response after JSON/schema validation fails.
        let raw = llm.chat_json(user_id, task, &current_prompt).await?;
        let result = serde_json::from_str::<T>(json_object_payload(&raw))
            .map_err(anyhow::Error::from)
            .and_then(|value| {
                validate(&value)?;
                Ok(value)
            });
        match result {
            Ok(value) => return Ok(value),
            Err(error) => {
                tracing::warn!(attempt, %error, "LLM JSON output failed validation");
                current_prompt = format!(
                    "{prompt}\n\nCORRECTION REQUIRED: the previous response failed validation: {error}. Return a new JSON object that fixes this error. Do not repeat the rejected value."
                );
                last_error = Some(error);
            }
        }
    }
    Err(last_error.expect("three validation attempts produce an error"))
}

fn canon_retry_prompt(base_prompt: &str, validation_error: &str) -> String {
    format!(
        "{base_prompt}\n\nCORRECTION REQUIRED: the previous response was rejected because {validation_error}. Return a completely new JSON object. Fix that validation error; copy every evidence excerpt directly from SOURCE."
    )
}

#[cfg(test)]
mod validated_json_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    struct InvalidThenValid {
        calls: AtomicUsize,
        saw_correction: AtomicBool,
    }

    #[async_trait::async_trait]
    impl LlmPort for InvalidThenValid {
        async fn chat_json(
            &self,
            _user_id: Uuid,
            _task: NovelLlmTask,
            prompt: &str,
        ) -> Result<String> {
            self.saw_correction
                .fetch_or(prompt.contains("CORRECTION REQUIRED"), Ordering::Relaxed);
            Ok(if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
                "{".into()
            } else {
                r#"{"ok":true}"#.into()
            })
        }
    }

    #[tokio::test]
    async fn retries_invalid_json_with_a_fresh_model_response() {
        let llm = InvalidThenValid {
            calls: AtomicUsize::new(0),
            saw_correction: AtomicBool::new(false),
        };
        let value: serde_json::Value = validated_json(
            &llm,
            Uuid::nil(),
            NovelLlmTask::CharacterExtraction,
            "prompt",
            |_| Ok(()),
        )
        .await
        .unwrap();

        assert_eq!(value["ok"], true);
        assert_eq!(llm.calls.load(Ordering::Relaxed), 2);
        assert!(llm.saw_correction.load(Ordering::Relaxed));
    }

    #[test]
    fn canon_retry_feedback_names_the_rule_without_echoing_model_output() {
        let prompt = canon_retry_prompt("BASE", "evidence excerpt is not source-verbatim");
        assert!(prompt.starts_with("BASE\n\nCORRECTION REQUIRED:"));
        assert!(prompt.contains("evidence excerpt is not source-verbatim"));
        assert!(prompt.contains("copy every evidence excerpt directly from SOURCE"));
    }
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

const MAX_TRANSLATION_BYTES: usize = 48_000;
const TRANSLATION_CHUNK_BYTES: usize = 12_000;
const TRANSLATION_TIMEOUT: Duration = Duration::from_secs(180);
const TRANSLATION_PROFILE: &str = "zh-cn-v1";

#[derive(Debug, thiserror::Error)]
pub enum TranslationError {
    #[error("Translation source must contain 1-48000 bytes")]
    Validation,
    #[error("Translation capacity is busy")]
    Capacity,
    #[error("Translation is already in progress")]
    InProgress { retry_after_seconds: u64 },
    #[error("Chapter not found")]
    ChapterNotFound,
    #[error("Translation source does not match the chapter")]
    SourceMismatch,
    #[error("Translation timed out")]
    Timeout,
    #[error("Translation repository failed")]
    Repository(#[source] anyhow::Error),
    #[error("Translation provider failed")]
    Provider(#[source] anyhow::Error),
}

pub struct TranslateChapterHandler {
    pub chapter_repo: Arc<dyn ChapterRepository>,
    pub translation_repo: Arc<dyn ChapterTranslationRepository>,
    pub translator: Arc<dyn TextTranslator>,
    pub permits: Arc<Semaphore>,
}

impl TranslateChapterHandler {
    pub async fn translate(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        source: &str,
    ) -> std::result::Result<String, TranslationError> {
        if source.trim().is_empty() || source.len() > MAX_TRANSLATION_BYTES {
            return Err(TranslationError::Validation);
        }
        let chapter = self
            .chapter_repo
            .find_by_number(novel_id, chapter_number)
            .await
            .map_err(TranslationError::Repository)?
            .ok_or(TranslationError::ChapterNotFound)?;
        if !chapter.content.starts_with(source) {
            return Err(TranslationError::SourceMismatch);
        }
        let source_hash = Sha256::digest(source.as_bytes()).to_vec();
        let key = ChapterTranslationKey {
            chapter_id: chapter.id,
            source_sha256: &source_hash,
            profile: TRANSLATION_PROFILE,
        };
        if let Some(content) = self
            .translation_repo
            .find_ready(key)
            .await
            .map_err(TranslationError::Repository)?
        {
            return Ok(content);
        }
        let _permit = self
            .permits
            .try_acquire()
            .map_err(|_| TranslationError::Capacity)?;
        let attempt = match self
            .translation_repo
            .begin(key)
            .await
            .map_err(TranslationError::Repository)?
        {
            BeginChapterTranslation::Ready(content) => return Ok(content),
            BeginChapterTranslation::Acquired { attempt } => attempt,
            BeginChapterTranslation::InProgress {
                retry_after_seconds,
            } => {
                return Err(TranslationError::InProgress {
                    retry_after_seconds,
                })
            }
        };

        let translated =
            match tokio::time::timeout(TRANSLATION_TIMEOUT, self.translate_chunks(user_id, source))
                .await
            {
                Ok(Ok(content)) => content,
                Ok(Err(error)) => {
                    self.fail_translation(key, attempt, "provider").await;
                    return Err(error);
                }
                Err(_) => {
                    self.fail_translation(key, attempt, "timeout").await;
                    return Err(TranslationError::Timeout);
                }
            };

        let completed = self
            .translation_repo
            .complete(key, attempt, &translated)
            .await
            .map_err(TranslationError::Repository)?;
        if completed {
            return Ok(translated);
        }
        if let Some(content) = self
            .translation_repo
            .find_ready(key)
            .await
            .map_err(TranslationError::Repository)?
        {
            return Ok(content);
        }
        Err(TranslationError::Repository(anyhow::anyhow!(
            "translation cache lease was lost before completion"
        )))
    }

    async fn fail_translation(
        &self,
        key: ChapterTranslationKey<'_>,
        attempt: i64,
        failure_code: &str,
    ) {
        match self.translation_repo.fail(key, attempt, failure_code).await {
            Ok(true) => {}
            Ok(false) => warn!(attempt, "translation cache lease was lost before failure"),
            Err(error) => warn!(
                ?error,
                attempt, "translation cache failure could not be recorded"
            ),
        }
    }

    async fn translate_chunks(
        &self,
        user_id: Uuid,
        source: &str,
    ) -> std::result::Result<String, TranslationError> {
        let mut translated = Vec::new();
        for chunk in split_translation_chunks(source, TRANSLATION_CHUNK_BYTES) {
            let value = self
                .translator
                .to_simplified_chinese(user_id, chunk)
                .await
                .map_err(TranslationError::Provider)?;
            let value = value.trim();
            if value.is_empty() {
                return Err(TranslationError::Provider(anyhow::anyhow!(
                    "translation provider returned empty text"
                )));
            }
            translated.push(value.to_owned());
        }
        Ok(translated.join("\n\n"))
    }
}

fn split_translation_chunks(source: &str, limit: usize) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < source.len() {
        let mut end = (start + limit).min(source.len());
        while end > start && !source.is_char_boundary(end) {
            end -= 1;
        }
        if end < source.len() {
            if let Some(paragraph_end) = source[start..end]
                .rfind("\n\n")
                .filter(|index| *index >= limit / 2)
            {
                end = start + paragraph_end + 2;
            }
        }
        if end == start {
            end = source[start..]
                .char_indices()
                .nth(1)
                .map_or(source.len(), |(offset, _)| start + offset);
        }
        chunks.push(&source[start..end]);
        start = end;
    }
    chunks
}

#[cfg(test)]
mod translation_tests {
    use super::*;
    use std::collections::hash_map::Entry;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct EchoTranslator {
        calls: AtomicUsize,
        delay: Duration,
    }

    struct FixedChapterRepository {
        id: Uuid,
        content: String,
    }

    impl FixedChapterRepository {
        fn new(content: String) -> Self {
            Self {
                id: Uuid::new_v4(),
                content,
            }
        }
    }

    enum CachedTranslationState {
        Translating { attempt: i64 },
        Ready(String),
    }

    type OwnedTranslationKey = (Uuid, Vec<u8>, String);

    #[derive(Default)]
    struct MemoryTranslationRepository {
        values: Mutex<HashMap<OwnedTranslationKey, CachedTranslationState>>,
    }

    impl MemoryTranslationRepository {
        fn owned_key(key: ChapterTranslationKey<'_>) -> OwnedTranslationKey {
            (
                key.chapter_id,
                key.source_sha256.to_vec(),
                key.profile.to_owned(),
            )
        }
    }

    #[async_trait::async_trait]
    impl ChapterTranslationRepository for MemoryTranslationRepository {
        async fn find_ready(&self, key: ChapterTranslationKey<'_>) -> Result<Option<String>> {
            let values = self.values.lock().unwrap();
            Ok(match values.get(&Self::owned_key(key)) {
                Some(CachedTranslationState::Ready(content)) => Some(content.clone()),
                _ => None,
            })
        }

        async fn begin(&self, key: ChapterTranslationKey<'_>) -> Result<BeginChapterTranslation> {
            let mut values = self.values.lock().unwrap();
            Ok(match values.entry(Self::owned_key(key)) {
                Entry::Vacant(entry) => {
                    entry.insert(CachedTranslationState::Translating { attempt: 1 });
                    BeginChapterTranslation::Acquired { attempt: 1 }
                }
                Entry::Occupied(entry) => match entry.get() {
                    CachedTranslationState::Ready(content) => {
                        BeginChapterTranslation::Ready(content.clone())
                    }
                    CachedTranslationState::Translating { .. } => {
                        BeginChapterTranslation::InProgress {
                            retry_after_seconds: 1,
                        }
                    }
                },
            })
        }

        async fn complete(
            &self,
            key: ChapterTranslationKey<'_>,
            attempt: i64,
            translated_content: &str,
        ) -> Result<bool> {
            let mut values = self.values.lock().unwrap();
            let Some(state) = values.get_mut(&Self::owned_key(key)) else {
                return Ok(false);
            };
            match state {
                CachedTranslationState::Translating {
                    attempt: current_attempt,
                } if *current_attempt == attempt => {
                    *state = CachedTranslationState::Ready(translated_content.to_owned());
                    Ok(true)
                }
                _ => Ok(false),
            }
        }

        async fn fail(
            &self,
            key: ChapterTranslationKey<'_>,
            attempt: i64,
            _failure_code: &str,
        ) -> Result<bool> {
            let mut values = self.values.lock().unwrap();
            let owned_key = Self::owned_key(key);
            let matches_attempt = matches!(
                values.get(&owned_key),
                Some(CachedTranslationState::Translating {
                    attempt: current_attempt
                }) if *current_attempt == attempt
            );
            if matches_attempt {
                values.remove(&owned_key);
            }
            Ok(matches_attempt)
        }
    }

    #[async_trait::async_trait]
    impl ChapterRepository for FixedChapterRepository {
        async fn replace_import_nodes(
            &self,
            _novel_id: Uuid,
            _attempt: i64,
            _nodes: &[(i32, String)],
        ) -> Result<bool> {
            Ok(false)
        }

        async fn find_by_novel(&self, _novel_id: Uuid) -> Result<Vec<Chapter>> {
            Ok(Vec::new())
        }

        async fn find_by_number(&self, novel_id: Uuid, number: i32) -> Result<Option<Chapter>> {
            let mut chapter = Chapter::new(novel_id, number, None, self.content.clone());
            chapter.id = self.id;
            Ok(Some(chapter))
        }

        async fn search_lore(
            &self,
            _novel_id: Uuid,
            _max_chapter: i32,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<LoreExcerpt>> {
            Ok(Vec::new())
        }
    }

    #[async_trait::async_trait]
    impl TextTranslator for EchoTranslator {
        async fn to_simplified_chinese(&self, _user_id: Uuid, source: &str) -> Result<String> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(self.delay).await;
            Ok(format!("译：{source}"))
        }
    }

    #[test]
    fn translation_chunks_preserve_utf8_source_and_prefer_paragraphs() {
        let source = format!("第一段。\n\n{}\n\nThe end.", "long text ".repeat(8));
        let chunks = split_translation_chunks(&source, 32);

        assert_eq!(chunks.concat(), source);
        assert!(chunks.len() > 1);
        assert!(chunks
            .iter()
            .all(|chunk| chunk.is_char_boundary(chunk.len())));
    }

    #[tokio::test]
    async fn translation_is_bounded_and_chunks_large_chapters() {
        let source = "English paragraph. ".repeat(900);
        let translator = Arc::new(EchoTranslator {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
        });
        let handler = TranslateChapterHandler {
            chapter_repo: Arc::new(FixedChapterRepository::new(source.clone())),
            translation_repo: Arc::new(MemoryTranslationRepository::default()),
            translator: translator.clone(),
            permits: Arc::new(Semaphore::new(1)),
        };

        assert!(matches!(
            handler.translate(Uuid::nil(), Uuid::nil(), 1, " \n ").await,
            Err(TranslationError::Validation)
        ));
        assert!(matches!(
            handler
                .translate(Uuid::nil(), Uuid::nil(), 1, "other text")
                .await,
            Err(TranslationError::SourceMismatch)
        ));
        let translated = handler
            .translate(Uuid::nil(), Uuid::nil(), 1, &source)
            .await
            .unwrap();

        assert!(translated.starts_with("译："));
        assert!(translator.calls.load(Ordering::Relaxed) > 1);
    }

    #[tokio::test]
    async fn completed_translation_is_reused_and_source_changes_miss() {
        let chapter = "Chapter one continues.".to_owned();
        let translator = Arc::new(EchoTranslator {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
        });
        let handler = TranslateChapterHandler {
            chapter_repo: Arc::new(FixedChapterRepository::new(chapter.clone())),
            translation_repo: Arc::new(MemoryTranslationRepository::default()),
            translator: translator.clone(),
            permits: Arc::new(Semaphore::new(2)),
        };

        let first = handler
            .translate(Uuid::nil(), Uuid::nil(), 1, "Chapter one")
            .await
            .unwrap();
        let cached = handler
            .translate(Uuid::nil(), Uuid::nil(), 1, "Chapter one")
            .await
            .unwrap();
        let changed = handler
            .translate(Uuid::nil(), Uuid::nil(), 1, &chapter)
            .await
            .unwrap();

        assert_eq!(first, cached);
        assert_ne!(first, changed);
        assert_eq!(translator.calls.load(Ordering::Relaxed), 2);
    }

    #[tokio::test]
    async fn concurrent_cold_requests_have_one_translation_owner() {
        let source = "Concurrent chapter".to_owned();
        let translator = Arc::new(EchoTranslator {
            calls: AtomicUsize::new(0),
            delay: Duration::from_millis(25),
        });
        let handler = TranslateChapterHandler {
            chapter_repo: Arc::new(FixedChapterRepository::new(source.clone())),
            translation_repo: Arc::new(MemoryTranslationRepository::default()),
            translator: translator.clone(),
            permits: Arc::new(Semaphore::new(2)),
        };

        let (first, second) = tokio::join!(
            handler.translate(Uuid::nil(), Uuid::nil(), 1, &source),
            handler.translate(Uuid::nil(), Uuid::nil(), 1, &source),
        );
        assert_eq!(first.is_ok() as u8 + second.is_ok() as u8, 1);
        assert!(matches!(
            first.as_ref().err().or_else(|| second.as_ref().err()),
            Some(TranslationError::InProgress { .. })
        ));
        assert_eq!(translator.calls.load(Ordering::Relaxed), 1);

        let cached = handler
            .translate(Uuid::nil(), Uuid::nil(), 1, &source)
            .await
            .unwrap();
        assert!(cached.starts_with("译："));
        assert_eq!(translator.calls.load(Ordering::Relaxed), 1);
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Persisted import chapters cannot be resumed")]
pub struct ImportChaptersUnusable;

#[derive(Debug, thiserror::Error)]
#[error("The retained source file is missing")]
pub struct ImportSourceMissing;

#[derive(Debug, thiserror::Error)]
pub enum ShelfMutationError {
    #[error("Novel not found")]
    NotFound,
    #[error("Novel privacy cleanup is unavailable")]
    PrivacyCleanup(#[source] anyhow::Error),
    #[error("Shelf mutation failed")]
    Repository(#[source] anyhow::Error),
}

#[derive(Debug, Clone, PartialEq)]
pub enum GameRuleTemplateRequest {
    Ready(GameRuleTemplate),
    InProgress { retry_after_seconds: u64 },
}

#[derive(Debug, thiserror::Error)]
pub enum GameRuleTemplateRequestError {
    #[error("Novel not found")]
    NovelNotFound,
    #[error("Canonical story model is unavailable")]
    CanonUnavailable,
    #[error("Game rule generation budget is exhausted")]
    BudgetExhausted,
    #[error("Game rule repository failed")]
    Repository(#[source] anyhow::Error),
}

const MAX_AVATARS_PER_NOVEL: usize = 30;
const MAX_CONCURRENT_AVATAR_REQUESTS: usize = 6;
const MAX_IMPORT_CHAPTERS: usize = 2_048;
const MAX_BOUNDARY_REPAIR_SEGMENTS: usize = 8;
const MAX_BOUNDARY_REPAIR_ROUNDS: usize = 3;
// 16 KiB canon chunks plus 24 KiB character scans keep the supported 5 MiB
// paste limit below this cap while rejecting the next whole-MiB tier.
const MAX_IMPORT_PROVIDER_CALLS: usize = 640;
const IMPORT_LEASE_HEARTBEAT: Duration = Duration::from_secs(30);
const IMPORT_RECOVERY_INTERVAL: Duration = Duration::from_secs(30);
const GAME_RULE_LEASE_HEARTBEAT: Duration = Duration::from_secs(30);

fn shared_avatar_admission() -> Arc<Semaphore> {
    static ADMISSION: OnceLock<Arc<Semaphore>> = OnceLock::new();
    ADMISSION
        .get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_AVATAR_REQUESTS)))
        .clone()
}

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

struct GameRuleLease {
    stop: Option<oneshot::Sender<()>>,
    lost: watch::Receiver<bool>,
}

impl GameRuleLease {
    fn start(
        repo: Arc<dyn CanonStoryModelRepository>,
        novel_id: Uuid,
        canon_model_version: i32,
        attempt: i64,
    ) -> Self {
        let (stop, mut stopped) = oneshot::channel();
        let (lost, receiver) = watch::channel(false);
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                let mut heartbeat = tokio::time::interval(GAME_RULE_LEASE_HEARTBEAT);
                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                heartbeat.tick().await;
                loop {
                    tokio::select! {
                        _ = &mut stopped => return,
                        _ = heartbeat.tick() => {
                            match repo
                                .renew_game_rule_generation(
                                    novel_id,
                                    canon_model_version,
                                    attempt,
                                )
                                .await
                            {
                                Ok(true) => {}
                                Ok(false) => {
                                    tracing::warn!(
                                        %novel_id,
                                        canon_model_version,
                                        attempt,
                                        "game rule generation lease was fenced"
                                    );
                                    let _ = lost.send(true);
                                    return;
                                }
                                Err(error) => {
                                    tracing::error!(
                                        %novel_id,
                                        canon_model_version,
                                        attempt,
                                        error = ?error,
                                        "game rule generation lease renewal failed"
                                    );
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

impl Drop for GameRuleLease {
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
    let calls = 4usize
        .saturating_add(character_scans)
        .saturating_add(canon_scans)
        .saturating_add(MAX_AVATARS_PER_NOVEL);
    (calls <= MAX_IMPORT_PROVIDER_CALLS)
        .then_some(())
        .ok_or(ImportBudgetExceeded)
}

fn prepare_import_command(
    cmd: ImportNovelCommand,
    source_retention_enabled: bool,
) -> Result<PreparedImport> {
    let raw_text = cmd
        .raw_content
        .ok_or_else(|| anyhow::anyhow!("Novel content is required"))?;
    let mut novel = Novel::create(cmd.user_id, cmd.title, cmd.author);
    if let Some(mode) = cmd.deviation_mode {
        novel.set_deviation_mode(mode);
    }
    let source_stage = cmd.source_bytes.is_some() && source_retention_enabled;
    let chapters = if source_stage {
        Vec::new()
    } else {
        let chapters = NovelParserService::parse_chapters(novel.id, &raw_text)?;
        ensure_import_budget(&chapters)?;
        chapters
    };
    Ok(PreparedImport {
        novel,
        chapters,
        source_bytes: cmd.source_bytes,
    })
}

impl NovelCommandHandler {
    fn try_admit_import(
        &self,
        user_id: Uuid,
    ) -> std::result::Result<ImportAdmission, ImportCapacityUnavailable> {
        try_import_admission(&self.import_permits, &self.active_import_users, user_id)
    }

    pub async fn remove_from_shelf(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<(), ShelfMutationError> {
        let attached = self
            .novel_repo
            .find_for_user(user_id, novel_id)
            .await
            .map_err(ShelfMutationError::Repository)?
            .is_some();
        if !attached {
            return Err(ShelfMutationError::NotFound);
        }
        if let Err(error) = self.privacy_cleanup.clear_novel(user_id, novel_id).await {
            if let Err(rollback_error) = self.privacy_cleanup.allow_novel(user_id, novel_id).await {
                tracing::warn!(error = ?rollback_error, %user_id, %novel_id, "novel privacy cleanup rollback failed");
            }
            return Err(ShelfMutationError::PrivacyCleanup(error));
        }
        if let Err(error) = self.novel_repo.detach_from_user(user_id, novel_id).await {
            if let Err(rollback_error) = self.privacy_cleanup.allow_novel(user_id, novel_id).await {
                tracing::error!(error = ?rollback_error, %user_id, %novel_id, "novel privacy cleanup rollback failed");
            }
            return Err(ShelfMutationError::Repository(error));
        }
        Ok(())
    }

    pub async fn attach_shared_novel(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        deviation_mode: DeviationMode,
    ) -> Result<(), ShelfMutationError> {
        let attached = self
            .novel_repo
            .attach_to_user(user_id, novel_id, deviation_mode)
            .await
            .map_err(ShelfMutationError::Repository)?;
        if !attached {
            return Err(ShelfMutationError::NotFound);
        }
        if let Err(error) = self.privacy_cleanup.allow_novel(user_id, novel_id).await {
            // Keep the durable shelf association. The cache tombstone fails
            // closed, and retrying this idempotent request repairs projection.
            return Err(ShelfMutationError::PrivacyCleanup(error));
        }
        Ok(())
    }

    pub async fn request_game_rule_template(
        self: &Arc<Self>,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> std::result::Result<GameRuleTemplateRequest, GameRuleTemplateRequestError> {
        let novel = self
            .novel_repo
            .find_by_id(novel_id)
            .await
            .map_err(GameRuleTemplateRequestError::Repository)?
            .ok_or(GameRuleTemplateRequestError::NovelNotFound)?;
        let model = self
            .canon_repo
            .find_latest(novel_id)
            .await
            .map_err(GameRuleTemplateRequestError::Repository)?
            .ok_or(GameRuleTemplateRequestError::CanonUnavailable)?;
        let attempt = match self
            .canon_repo
            .begin_game_rule_generation(novel_id, model.model_version)
            .await
            .map_err(GameRuleTemplateRequestError::Repository)?
        {
            BeginGameRuleGeneration::Ready(template) => {
                return Ok(GameRuleTemplateRequest::Ready(template));
            }
            BeginGameRuleGeneration::InProgress {
                retry_after_seconds,
            } => {
                return Ok(GameRuleTemplateRequest::InProgress {
                    retry_after_seconds,
                });
            }
            BeginGameRuleGeneration::Exhausted => {
                return Err(GameRuleTemplateRequestError::BudgetExhausted);
            }
            BeginGameRuleGeneration::Acquired { attempt } => attempt,
        };

        self.spawn_game_rule_generation(user_id, novel, model, attempt);
        Ok(GameRuleTemplateRequest::InProgress {
            retry_after_seconds: 2,
        })
    }

    fn spawn_game_rule_generation(
        self: &Arc<Self>,
        user_id: Uuid,
        novel: Novel,
        model: crate::domain::entities::canon_story_model::CanonStoryModel,
        attempt: i64,
    ) {
        let handler = Arc::clone(self);
        let current_span = tracing::Span::current();
        tokio::spawn(
            async move {
                handler
                    .finish_claimed_game_rule_generation(user_id, novel, model, attempt)
                    .await;
            }
            .instrument(current_span),
        );
    }

    async fn finish_claimed_game_rule_generation(
        &self,
        user_id: Uuid,
        novel: Novel,
        model: crate::domain::entities::canon_story_model::CanonStoryModel,
        attempt: i64,
    ) {
        let novel_id = novel.id;
        let generation_started = Instant::now();
        match self
            .generate_claimed_game_rule_template(user_id, &novel, &model, attempt)
            .await
        {
            Ok(_) => {
                info!(
                    %novel_id,
                    canon_model_version = model.model_version,
                    attempt,
                    elapsed_ms = generation_started.elapsed().as_millis(),
                    "game rule template generation completed"
                );
            }
            Err(error) => {
                tracing::error!(
                    %novel_id,
                    canon_model_version = model.model_version,
                    attempt,
                    elapsed_ms = generation_started.elapsed().as_millis(),
                    error = ?error,
                    "game rule generation failed"
                );
                if let Err(failure_error) = self
                    .canon_repo
                    .fail_game_rule_generation(
                        novel_id,
                        model.model_version,
                        attempt,
                        "generation_failed",
                    )
                    .await
                {
                    tracing::error!(
                        %novel_id,
                        canon_model_version = model.model_version,
                        attempt,
                        error = ?failure_error,
                        "failed to persist game rule generation failure"
                    );
                }
            }
        }
    }

    async fn generate_claimed_game_rule_template(
        &self,
        user_id: Uuid,
        novel: &Novel,
        model: &crate::domain::entities::canon_story_model::CanonStoryModel,
        attempt: i64,
    ) -> Result<GameRuleTemplate> {
        let prompt = game_rule_generator::build_prompt(&novel.title, model)?;
        let allowed_source_chapters = game_rule_generator::source_chapters(model);
        let mut lease = GameRuleLease::start(
            self.canon_repo.clone(),
            novel.id,
            model.model_version,
            attempt,
        );
        anyhow::ensure!(
            prompt.len() <= game_rule_generator::MAX_GAME_RULE_PROMPT_BYTES,
            "game rule prompt exceeds its byte budget"
        );
        let raw = lease
            .run(
                self.llm
                    .chat_json(user_id, NovelLlmTask::GameRuleGeneration, &prompt),
            )
            .await
            .ok_or_else(|| anyhow::anyhow!("game rule generation lease was lost"))??;
        let template = game_rule_generator::parse_template(
            &raw,
            novel.id,
            model.model_version,
            &allowed_source_chapters,
        )?;
        let completed = lease
            .run(
                self.canon_repo
                    .complete_game_rule_generation(&template, attempt),
            )
            .await
            .ok_or_else(|| anyhow::anyhow!("game rule generation lease was lost"))??;
        anyhow::ensure!(completed, "game rule generation completion was fenced");
        lease.stop();
        Ok(template)
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
        let mut ids = self.handle_import_batch(vec![cmd]).await?;
        Ok(ids.pop().expect("a non-empty import batch returns one id"))
    }

    /// Atomically accept a bounded set of independent Novel aggregates while
    /// consuming one admission slot. Only the first durable job is claimed
    /// here; the existing recovery loop claims the remaining pending jobs.
    pub async fn handle_import_batch(
        self: &Arc<Self>,
        commands: Vec<ImportNovelCommand>,
    ) -> Result<Vec<Uuid>> {
        anyhow::ensure!(
            !commands.is_empty() && commands.len() <= MAX_BATCH_IMPORTS,
            "import batch must contain 1-{MAX_BATCH_IMPORTS} novels"
        );
        let user_id = commands[0].user_id;
        anyhow::ensure!(
            commands.iter().all(|command| command.user_id == user_id),
            "import batch must belong to one user"
        );
        info!(batch_size = commands.len(), %user_id, "accepting novel import batch");
        let admission = self.try_admit_import(user_id)?;
        let source_retention_enabled = self.source_storage.is_some();
        let mut prepared = tokio::task::spawn_blocking(move || {
            commands
                .into_iter()
                .map(|command| prepare_import_command(command, source_retention_enabled))
                .collect::<Result<Vec<_>>>()
        })
        .await
        .map_err(|error| anyhow::anyhow!("chapter parser task failed: {error}"))??;

        for import in &mut prepared {
            retain_source_file(
                &mut import.novel,
                import.source_bytes.take(),
                self.source_storage.as_deref(),
                self.source_deletions.as_ref(),
            )
            .await?;
        }
        let imports = prepared
            .into_iter()
            .map(|prepared| (prepared.novel, prepared.chapters))
            .collect::<Vec<_>>();
        let novel_ids = imports
            .iter()
            .map(|(novel, _)| novel.id)
            .collect::<Vec<_>>();
        self.novel_repo.create_import_batch(&imports).await?;

        let first_novel_id = novel_ids[0];
        match self.novel_repo.claim_import(first_novel_id, user_id).await {
            Ok(Some(claim)) => self.spawn_claimed_import(claim, admission),
            Ok(None) => {
                info!(novel_id = %first_novel_id, "durable novel import was claimed by another worker");
            }
            Err(error) => {
                error!(error = ?error, novel_id = %first_novel_id, "durable novel import awaits recovery");
            }
        }
        Ok(novel_ids)
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
                // Core import completion is durable. Cosmetic work no longer
                // holds import admission, but it has its own service-wide cap.
                drop(admission);
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
        let chapters = if matches!(claim.stage, ImportStage::Source | ImportStage::Chapters) {
            self.repair_chapter_boundaries(chapters, claim).await?
        } else {
            chapters
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

    async fn repair_chapter_boundaries(
        &self,
        mut chapters: Vec<Chapter>,
        claim: &ImportClaim,
    ) -> Result<Vec<Chapter>> {
        let mut repaired_segments = 0usize;
        for round in 1..=MAX_BOUNDARY_REPAIR_ROUNDS {
            let indexes = chapter_boundary_detector::suspicious_chapter_indexes(&chapters);
            if indexes.is_empty() {
                return Ok(chapters);
            }
            repaired_segments = repaired_segments.saturating_add(indexes.len());
            if repaired_segments > MAX_BOUNDARY_REPAIR_SEGMENTS {
                return Err(ImportBudgetExceeded.into());
            }
            info!(
                novel_id = %claim.novel_id,
                round,
                segments = indexes.len(),
                "repairing suspicious chapter boundaries"
            );
            let user_id = claim.user_id;
            let results = stream::iter(indexes)
                .map(|index| {
                    let llm = self.llm.clone();
                    let chapter = chapters[index].clone();
                    let expected_boundaries =
                        chapter_boundary_detector::expected_boundary_count(&chapters, index);
                    async move {
                        let prompt =
                            chapter_boundary_detector::build_prompt(&chapter, expected_boundaries)?;
                        let detection: chapter_boundary_detector::ChapterBoundaryDetection =
                            validated_json(
                                llm.as_ref(),
                                user_id,
                                NovelLlmTask::ChapterBoundaryDetection,
                                &prompt,
                                |result| {
                                    chapter_boundary_detector::validate_detection(
                                        result,
                                        &chapter.content,
                                        expected_boundaries,
                                    )
                                    .map_err(Into::into)
                                },
                            )
                            .await?;
                        let parts = chapter_boundary_detector::split_chapter(
                            &chapter,
                            &detection,
                            expected_boundaries,
                        )?;
                        Ok::<_, anyhow::Error>((index, parts))
                    }
                })
                .buffer_unordered(2)
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>>>()?;
            let repairs = results.into_iter().collect::<HashMap<_, _>>();
            let mut repaired = Vec::new();
            for (index, chapter) in chapters.into_iter().enumerate() {
                match repairs.get(&index) {
                    Some(parts) => repaired.extend(parts.iter().cloned()),
                    None => repaired.push(chapter),
                }
            }
            for (index, chapter) in repaired.iter_mut().enumerate() {
                chapter.chapter_number = i32::try_from(index + 1)
                    .map_err(|_| anyhow::anyhow!("chapter number exceeds i32"))?;
            }
            if !self
                .novel_repo
                .replace_import_chapters(claim.novel_id, claim.attempt, &repaired)
                .await?
            {
                return Err(ImportLeaseLost.into());
            }
            info!(
                novel_id = %claim.novel_id,
                round,
                chapters = repaired.len(),
                "repaired chapter boundaries committed"
            );
            chapters = repaired;
        }
        if chapter_boundary_detector::suspicious_chapter_indexes(&chapters).is_empty() {
            Ok(chapters)
        } else {
            anyhow::bail!("suspicious chapter boundaries remain after repair limit")
        }
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
        let mut base_extraction: ExtractionResult = validated_json(
            self.llm.as_ref(),
            claim.user_id,
            NovelLlmTask::CharacterExtraction,
            &prompt,
            |result| validate_extraction(result).map_err(Into::into),
        )
        .await?;

        let mut chunk_extractions = Vec::new();
        if needs_chunk_scan(chapters) {
            let scans = build_scan_plan(chapters);
            let user_id = claim.user_id;
            let results = stream::iter(scans.into_iter().enumerate())
                .map(|(index, chunk)| {
                    let llm = self.llm.clone();
                    let title = title.clone();
                    async move {
                        let prompt = build_chunk_extraction_prompt(&title, &chunk, index);
                        let result: ChunkExtractionResult = validated_json(
                            llm.as_ref(),
                            user_id,
                            NovelLlmTask::CharacterExtraction,
                            &prompt,
                            |result| validate_chunk_extraction(result).map_err(Into::into),
                        )
                        .await?;
                        Ok::<_, anyhow::Error>((index, result))
                    }
                })
                .buffer_unordered(3)
                .collect::<Vec<_>>()
                .await;
            let mut results = results.into_iter().collect::<Result<Vec<_>>>()?;
            results.sort_by_key(|(index, _)| *index);
            chunk_extractions = results
                .into_iter()
                .map(|(_, extraction)| extraction)
                .collect();
            // The representative sample spans unrelated chapters. Use it for
            // global metadata only; source-ordered chunks own character facts.
            base_extraction.characters.clear();
            base_extraction.relationships.clear();
        }
        let extraction = merge_extractions(base_extraction, chunk_extractions);
        validate_extraction(&extraction)?;

        // 保存角色
        let characters: Vec<Character> = extraction
            .characters
            .iter()
            .filter_map(|ec| {
                let first_appearance =
                    find_first_appearance(ec, &extraction.characters, chapters);
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
        let chapter_numbers = chapters
            .iter()
            .map(|chapter| chapter.chapter_number)
            .collect::<Vec<_>>();
        let detection: node_detector::NodeDetectionResult = validated_json(
            self.llm.as_ref(),
            claim.user_id,
            NovelLlmTask::NarrativeNodeDetection,
            &node_prompt,
            |result| {
                node_detector::validate_detection(result, chapter_numbers.iter().copied())
                    .map_err(Into::into)
            },
        )
        .await?;
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
            let user_id = claim.user_id;
            let results = stream::iter(chunks.into_iter().enumerate())
                .map(|(position, chunk)| {
                    let llm = self.llm.clone();
                    let canon_repo = self.canon_repo.clone();
                    let title = novel.title.clone();
                    let novel_id = novel.id;
                    let import_attempt = claim.attempt;
                    async move {
                        let chunk_index = i32::try_from(chunk.chunk_index)
                            .map_err(|_| anyhow::anyhow!("canon chunk index exceeds i32"))?;
                        if let Some(checkpoint) = canon_repo
                            .find_import_checkpoint(
                                novel_id,
                                1,
                                canon_story_extractor::CANON_CHUNK_PROMPT_VERSION,
                                chunk.chapter_number,
                                chunk_index,
                                &chunk.content,
                            )
                            .await?
                        {
                            match canon_story_extractor::parse_chunk(&checkpoint, &chunk).and_then(
                                |mut extraction| {
                                    canon_story_extractor::canonicalize_character_references(
                                        &mut extraction,
                                        characters,
                                    )?;
                                    Ok(extraction)
                                },
                            ) {
                                Ok(extraction) => {
                                    info!(
                                        novel_id = %novel_id,
                                        chapter = chunk.chapter_number,
                                        chunk = chunk.chunk_index,
                                        "resumed canonical extraction checkpoint"
                                    );
                                    return Ok::<_, anyhow::Error>((position, chunk, extraction));
                                }
                                Err(error) => tracing::warn!(
                                    novel_id = %novel_id,
                                    chapter = chunk.chapter_number,
                                    chunk = chunk.chunk_index,
                                    %error,
                                    "discarding invalid canonical extraction checkpoint"
                                ),
                            }
                        }
                        let base_prompt =
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
                        let mut prompt = base_prompt.clone();
                        for attempt in 0..3 {
                            let raw = llm
                                .chat_json(user_id, NovelLlmTask::CanonExtraction, &prompt)
                                .await?;
                            match canon_story_extractor::parse_chunk(&raw, &chunk).and_then(
                                |mut extraction| {
                                    canon_story_extractor::canonicalize_character_references(
                                        &mut extraction,
                                        characters,
                                    )?;
                                    Ok(extraction)
                                },
                            ) {
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
                                        prompt = canon_retry_prompt(&base_prompt, &error.to_string());
                                    }
                                    last_error = Some(error);
                                }
                            }
                        }
                        let extraction = extraction.ok_or_else(|| {
                            anyhow::anyhow!(
                                "canonical extraction failed validation after 3 attempts at chapter {} chunk {}: {:?}",
                                chunk.chapter_number,
                                chunk.chunk_index,
                                last_error,
                            )
                        })?;
                        let extraction_json = serde_json::to_string(&extraction)?;
                        if !canon_repo
                            .save_import_checkpoint(
                                CanonExtractionCheckpoint {
                                    novel_id,
                                    model_version: 1,
                                    prompt_version:
                                        canon_story_extractor::CANON_CHUNK_PROMPT_VERSION,
                                    chapter_number: chunk.chapter_number,
                                    chunk_index,
                                    is_final: chunk.is_final,
                                    source_content: &chunk.content,
                                    extraction_json: &extraction_json,
                                },
                                import_attempt,
                            )
                            .await?
                        {
                            return Err(ImportLeaseLost.into());
                        }
                        Ok::<_, anyhow::Error>((position, chunk, extraction))
                    }
                })
                .buffer_unordered(4)
                .collect::<Vec<_>>()
                .await;
            let mut extracted = results.into_iter().collect::<Result<Vec<_>>>()?;
            extracted.sort_by_key(|(position, _, _)| *position);
            let mut extracted = extracted
                .into_iter()
                .map(|(_, chunk, extraction)| (chunk, extraction))
                .collect::<Vec<_>>();
            if let Some(prompt) =
                canon_story_extractor::build_event_selection_prompt(&novel.title, &extracted)
            {
                let candidate_count = extracted
                    .iter()
                    .map(|(_, extraction)| extraction.events.len())
                    .sum();
                let final_chunk = &extracted.last().expect("canon chunks are non-empty").0;
                let chapter_number = final_chunk.chapter_number;
                let chunk_index = i32::try_from(final_chunk.chunk_index)
                    .map_err(|_| anyhow::anyhow!("canon chunk index exceeds i32"))?;
                let checkpoint = self
                    .canon_repo
                    .find_import_checkpoint(
                        novel.id,
                        1,
                        canon_story_extractor::CANON_EVENT_SELECTION_PROMPT_VERSION,
                        chapter_number,
                        chunk_index,
                        &prompt,
                    )
                    .await?;
                let mut checkpointed = false;
                let mut selection = match checkpoint {
                    Some(raw) => {
                        match canon_story_extractor::parse_event_selection(&raw, candidate_count) {
                            Ok(selection) => {
                                checkpointed = true;
                                info!(
                                    novel_id = %novel.id,
                                    "resumed canonical event selection checkpoint"
                                );
                                Some(selection)
                            }
                            Err(error) => {
                                tracing::warn!(
                                    novel_id = %novel.id,
                                    %error,
                                    "discarding invalid canonical event selection checkpoint"
                                );
                                None
                            }
                        }
                    }
                    None => None,
                };
                if selection.is_none() {
                    for schema_attempt in 1..=2 {
                        let raw = self
                            .llm
                            .chat_json(claim.user_id, NovelLlmTask::CanonExtraction, &prompt)
                            .await?;
                        match canon_story_extractor::parse_event_selection(&raw, candidate_count) {
                            Ok(parsed) => {
                                selection = Some(parsed);
                                break;
                            }
                            Err(error) if schema_attempt == 1 => {
                                tracing::warn!(
                                    novel_id = %novel.id,
                                    %error,
                                    "retrying canonical event grouping after invalid schema"
                                );
                            }
                            Err(error) => return Err(error.into()),
                        }
                    }
                }
                let selection = selection
                    .ok_or_else(|| anyhow::anyhow!("canonical event selection is missing"))?;
                if !checkpointed {
                    let selection_json = serde_json::to_string(&selection)?;
                    if !self
                        .canon_repo
                        .save_import_checkpoint(
                            CanonExtractionCheckpoint {
                                novel_id: novel.id,
                                model_version: 1,
                                prompt_version:
                                    canon_story_extractor::CANON_EVENT_SELECTION_PROMPT_VERSION,
                                chapter_number,
                                chunk_index,
                                is_final: true,
                                source_content: &prompt,
                                extraction_json: &selection_json,
                            },
                            claim.attempt,
                        )
                        .await?
                    {
                        return Err(ImportLeaseLost.into());
                    }
                }
                canon_story_extractor::apply_event_selection(&mut extracted, &selection)?;
            }
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
        let avatar_admission = shared_avatar_admission();
        stream::iter(avatar_jobs)
            .for_each_concurrent(3, |(character_id, appearance)| {
                let character_repo = self.character_repo.clone();
                let image_client = self.image_client.clone();
                let avatar_admission = avatar_admission.clone();
                async move {
                    if let Err(error) = Self::generate_avatar(
                        character_id,
                        &appearance,
                        character_repo,
                        image_client,
                        avatar_admission,
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
        avatar_admission: Arc<Semaphore>,
    ) -> Result<()> {
        let _permit = avatar_admission
            .acquire_owned()
            .await
            .map_err(|_| anyhow::anyhow!("avatar admission closed"))?;
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct AvatarRepository;

    #[async_trait::async_trait]
    impl CharacterRepository for AvatarRepository {
        async fn replace_import(
            &self,
            _novel_id: Uuid,
            _attempt: i64,
            _characters: &[Character],
            _relationships: &[CharacterRelationshipRecord],
        ) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn find_by_novel(&self, _novel_id: Uuid) -> Result<Vec<Character>> {
            unreachable!("unused test repository method")
        }

        async fn find_by_id(&self, _id: Uuid) -> Result<Option<Character>> {
            unreachable!("unused test repository method")
        }

        async fn set_avatar(&self, _character_id: Uuid, _avatar_url: &str) -> Result<()> {
            Ok(())
        }

        async fn find_relationships(
            &self,
            _novel_id: Uuid,
        ) -> Result<Vec<CharacterRelationshipRecord>> {
            unreachable!("unused test repository method")
        }
    }

    #[derive(Default)]
    struct CountingImage {
        active: AtomicUsize,
        maximum: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl ImagePort for CountingImage {
        async fn generate(&self, _prompt: &str) -> Result<String> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(20)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok("https://example.invalid/avatar.png".into())
        }
    }

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
    fn durable_completion_releases_import_admission_before_optional_work() {
        let permits = Arc::new(Semaphore::new(1));
        let users = Arc::new(Mutex::new(HashSet::new()));
        let user_id = Uuid::new_v4();
        let admission = try_import_admission(&permits, &users, user_id).unwrap();

        drop(admission);
        assert_eq!(permits.available_permits(), 1);
        assert!(!users.lock().unwrap().contains(&user_id));
        let next = try_import_admission(&permits, &users, user_id);
        assert!(next.is_ok());
    }

    #[tokio::test]
    async fn avatar_requests_share_a_service_wide_admission_cap() {
        let permits = Arc::new(Semaphore::new(2));
        let repository: Arc<dyn CharacterRepository> = Arc::new(AvatarRepository);
        let image = Arc::new(CountingImage::default());
        let results = stream::iter(0..6)
            .map(|_| {
                let repository = repository.clone();
                let image_client: Arc<dyn ImagePort> = image.clone();
                let permits = permits.clone();
                async move {
                    NovelCommandHandler::generate_avatar(
                        Uuid::new_v4(),
                        "silver hair",
                        repository,
                        image_client,
                        permits,
                    )
                    .await
                }
            })
            .buffer_unordered(6)
            .collect::<Vec<_>>()
            .await;

        assert!(results.into_iter().all(|result| result.is_ok()));
        assert_eq!(image.maximum.load(Ordering::SeqCst), 2);
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
    #[error("Reader identity is unavailable at current progress")]
    IdentityUnavailable,
    #[error("{0}")]
    Validation(String),
    #[error("Reading progress operation failed")]
    Internal(#[source] anyhow::Error),
}

/// Public character contract bounded by the caller's persisted reading progress.
/// The stored `Character` remains canonical; this DTO is the only shape exposed
/// by progress-aware character reads.
#[derive(Debug, Clone, Serialize)]
pub struct ProgressBoundCharacter {
    pub id: Uuid,
    pub novel_id: Uuid,
    pub name: String,
    pub first_appearance_chapter: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aliases: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<CharacterRole>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub personality: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaking_style: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub appearance: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_status: Option<AvatarStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub persona_source_chapter_high_water: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<DateTime<Utc>>,
}

impl ProgressBoundCharacter {
    fn partial(character: &Character, first_appearance_chapter: i32) -> Self {
        Self {
            id: character.id,
            novel_id: character.novel_id,
            name: character.name.clone(),
            first_appearance_chapter,
            aliases: None,
            role: None,
            description: None,
            personality: None,
            background: None,
            speaking_style: None,
            appearance: None,
            avatar_url: None,
            avatar_status: None,
            persona_source_chapter_high_water: None,
            created_at: None,
            updated_at: None,
        }
    }

    fn full(character: &Character, first_appearance_chapter: i32, total_chapters: i32) -> Self {
        Self {
            id: character.id,
            novel_id: character.novel_id,
            name: character.name.clone(),
            first_appearance_chapter,
            aliases: Some(character.aliases.clone()),
            role: Some(character.role.clone()),
            description: character.description.clone(),
            personality: character.personality.clone(),
            background: character.background.clone(),
            speaking_style: character.speaking_style.clone(),
            appearance: character.appearance.clone(),
            avatar_url: character.avatar_url.clone(),
            avatar_status: Some(character.avatar_status.clone()),
            persona_source_chapter_high_water: Some(total_chapters),
            created_at: Some(character.created_at),
            updated_at: Some(character.updated_at),
        }
    }
}

fn persona_is_complete(novel: &Novel, current_chapter: i32) -> bool {
    novel.status == NovelStatus::Ready
        && novel.total_chapters > 0
        && current_chapter == novel.total_chapters
}

fn progress_bound_character(
    character: &Character,
    novel: &Novel,
    current_chapter: i32,
    canonical_name_source_proven: bool,
) -> Option<ProgressBoundCharacter> {
    if !character_name_is_canonical(&character.name) {
        return None;
    }
    let first_appearance_chapter = character
        .first_appearance_chapter
        .filter(|chapter| (1..=current_chapter).contains(chapter))?;

    if persona_is_complete(novel, current_chapter) {
        Some(ProgressBoundCharacter::full(
            character,
            first_appearance_chapter,
            novel.total_chapters,
        ))
    } else {
        canonical_name_source_proven
            .then(|| ProgressBoundCharacter::partial(character, first_appearance_chapter))
    }
}

fn known_character_names(characters: &[Character]) -> HashSet<&str> {
    characters
        .iter()
        .flat_map(|character| {
            std::iter::once(character.name.as_str())
                .chain(character.aliases.iter().map(String::as_str))
        })
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect()
}

fn canonical_name_is_source_proven(
    character: &Character,
    known_names: &HashSet<&str>,
    source_chapters: &HashMap<i32, Chapter>,
) -> bool {
    character
        .first_appearance_chapter
        .and_then(|chapter| source_chapters.get(&chapter))
        .is_some_and(|chapter| text_contains_name(&chapter.content, &character.name, known_names))
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
            .find_for_user(user_id, novel_id)
            .await
            .map_err(ReadingProgressError::Internal)?
        {
            Some(novel) => Ok(novel),
            _ => Err(ReadingProgressError::NotFound),
        }
    }

    async fn progress_for_novel(
        &self,
        user_id: Uuid,
        novel: &Novel,
    ) -> std::result::Result<ReadingProgressRecord, ReadingProgressError> {
        let progress = self.persisted_progress_for_novel(user_id, novel).await?;
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

    async fn persisted_progress_for_novel(
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
        Ok(progress)
    }

    async fn source_chapters_for(
        &self,
        novel_id: Uuid,
        chapter_numbers: HashSet<i32>,
    ) -> std::result::Result<HashMap<i32, Chapter>, ReadingProgressError> {
        let mut chapter_numbers = chapter_numbers.into_iter().collect::<Vec<_>>();
        chapter_numbers.sort_unstable();
        let mut chapters = HashMap::with_capacity(chapter_numbers.len());
        for chapter_number in chapter_numbers {
            if let Some(chapter) = self
                .chapter_repo
                .find_by_number(novel_id, chapter_number)
                .await
                .map_err(ReadingProgressError::Internal)?
            {
                chapters.insert(chapter_number, chapter);
            }
        }
        Ok(chapters)
    }

    pub async fn get(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> std::result::Result<ReadingProgressRecord, ReadingProgressError> {
        let novel = self.owned_novel(user_id, novel_id).await?;
        let initial_progress = self.progress_for_novel(user_id, &novel).await?;
        let identity_source = if let Some(character_id) = initial_progress.reader_character_id {
            let characters = self
                .character_repo
                .find_by_novel(novel_id)
                .await
                .map_err(ReadingProgressError::Internal)?;
            let chapters = if persona_is_complete(&novel, initial_progress.current_chapter) {
                HashMap::new()
            } else {
                let chapter_numbers = characters
                    .iter()
                    .find(|character| character.id == character_id)
                    .and_then(|character| character.first_appearance_chapter)
                    .filter(|chapter| (1..=initial_progress.current_chapter).contains(chapter))
                    .into_iter()
                    .collect();
                self.source_chapters_for(novel_id, chapter_numbers).await?
            };
            Some((character_id, characters, chapters))
        } else {
            None
        };

        // This is deliberately the final await. The response is projected only
        // from this persisted snapshot and the source evidence loaded above.
        let progress = self.persisted_progress_for_novel(user_id, &novel).await?;
        if progress.current_chapter != initial_progress.current_chapter
            || progress.reader_identity != initial_progress.reader_identity
            || progress.reader_identity_type != initial_progress.reader_identity_type
            || progress.reader_character_id != initial_progress.reader_character_id
        {
            return Err(ReadingProgressError::Internal(anyhow::anyhow!(
                "persisted reading progress changed during validation"
            )));
        }

        let persisted_identity = normalize_identity_name(progress.reader_identity.clone())
            .map_err(|_| ReadingProgressError::IdentityUnavailable)?;
        if persisted_identity != progress.reader_identity {
            return Err(ReadingProgressError::IdentityUnavailable);
        }
        match ReaderIdentityType::from_str(&progress.reader_identity_type) {
            Some(ReaderIdentityType::Self_) if progress.reader_character_id.is_none() => {}
            Some(ReaderIdentityType::Character) => {
                let character_id = progress
                    .reader_character_id
                    .ok_or(ReadingProgressError::IdentityUnavailable)?;
                let Some((loaded_id, characters, source_chapters)) = identity_source.as_ref()
                else {
                    return Err(ReadingProgressError::IdentityUnavailable);
                };
                let Some(character) = (*loaded_id == character_id)
                    .then(|| {
                        characters
                            .iter()
                            .find(|character| character.id == character_id)
                    })
                    .flatten()
                else {
                    return Err(ReadingProgressError::IdentityUnavailable);
                };
                if character.novel_id != novel_id
                    || validate_character_appearance(
                        character.first_appearance_chapter,
                        progress.current_chapter,
                    )
                    .is_err()
                    || !character_name_is_canonical(&character.name)
                {
                    return Err(ReadingProgressError::IdentityUnavailable);
                }
                let known_names = known_character_names(characters);
                if !persona_is_complete(&novel, progress.current_chapter)
                    && !canonical_name_is_source_proven(character, &known_names, source_chapters)
                {
                    return Err(ReadingProgressError::IdentityUnavailable);
                }
                if progress.reader_identity.as_deref() != Some(character.name.as_str()) {
                    return Err(ReadingProgressError::IdentityUnavailable);
                }
            }
            _ => {
                return Err(ReadingProgressError::IdentityUnavailable);
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
    ) -> std::result::Result<Vec<ProgressBoundCharacter>, ReadingProgressError> {
        let novel = self.owned_novel(user_id, novel_id).await?;
        let characters = self
            .character_repo
            .find_by_novel(novel_id)
            .await
            .map_err(ReadingProgressError::Internal)?;
        let validated_progress = self.progress_for_novel(user_id, &novel).await?;
        let source_chapters = if persona_is_complete(&novel, validated_progress.current_chapter) {
            HashMap::new()
        } else {
            let chapter_numbers = characters
                .iter()
                .filter_map(|character| character.first_appearance_chapter)
                .filter(|chapter| (1..=validated_progress.current_chapter).contains(chapter))
                .collect();
            self.source_chapters_for(novel_id, chapter_numbers).await?
        };
        // Keep this as the final await so a concurrent rewind cannot reuse an
        // earlier complete snapshot after persona/source IO finishes.
        let progress = self.persisted_progress_for_novel(user_id, &novel).await?;
        if progress.current_chapter != validated_progress.current_chapter {
            return Err(ReadingProgressError::Internal(anyhow::anyhow!(
                "persisted reading progress changed during character validation"
            )));
        }
        let known_names = known_character_names(&characters);
        Ok(characters
            .iter()
            .filter_map(|character| {
                progress_bound_character(
                    character,
                    &novel,
                    progress.current_chapter,
                    canonical_name_is_source_proven(character, &known_names, &source_chapters),
                )
            })
            .collect())
    }

    pub async fn get_available_character(
        &self,
        user_id: Uuid,
        character_id: Uuid,
    ) -> std::result::Result<ProgressBoundCharacter, ReadingProgressError> {
        let requested = self
            .character_repo
            .find_by_id(character_id)
            .await
            .map_err(ReadingProgressError::Internal)?
            .ok_or(ReadingProgressError::CharacterNotFound)?;
        let novel = self.owned_novel(user_id, requested.novel_id).await?;
        let characters = self
            .character_repo
            .find_by_novel(requested.novel_id)
            .await
            .map_err(ReadingProgressError::Internal)?;
        let character = characters
            .iter()
            .find(|character| character.id == character_id)
            .ok_or(ReadingProgressError::CharacterNotFound)?;
        let validated_progress = self.progress_for_novel(user_id, &novel).await?;
        let source_chapters = if persona_is_complete(&novel, validated_progress.current_chapter) {
            HashMap::new()
        } else {
            let chapter_numbers = character
                .first_appearance_chapter
                .filter(|chapter| (1..=validated_progress.current_chapter).contains(chapter))
                .into_iter()
                .collect();
            self.source_chapters_for(novel.id, chapter_numbers).await?
        };
        // Character and source reads are complete; this persisted progress
        // snapshot is the sole authority used by the pure response projection.
        let progress = self.persisted_progress_for_novel(user_id, &novel).await?;
        if progress.current_chapter != validated_progress.current_chapter {
            return Err(ReadingProgressError::Internal(anyhow::anyhow!(
                "persisted reading progress changed during character validation"
            )));
        }
        let known_names = known_character_names(&characters);
        progress_bound_character(
            character,
            &novel,
            progress.current_chapter,
            canonical_name_is_source_proven(character, &known_names, &source_chapters),
        )
        .ok_or(ReadingProgressError::CharacterNotFound)
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
                let characters = self
                    .character_repo
                    .find_by_novel(novel_id)
                    .await
                    .map_err(ReadingProgressError::Internal)?;
                let character = characters
                    .iter()
                    .find(|character| character.id == character_id)
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
                let chapter_numbers = character
                    .first_appearance_chapter
                    .filter(|chapter| (1..=novel.total_chapters).contains(chapter))
                    .into_iter()
                    .collect();
                let source_chapters = self.source_chapters_for(novel_id, chapter_numbers).await?;
                let known_names = known_character_names(&characters);
                if !canonical_name_is_source_proven(character, &known_names, &source_chapters) {
                    return Err(ReadingProgressError::Validation(
                        "reader character name is not source-proven".into(),
                    ));
                }
                (
                    normalize_identity_name(Some(character.name.clone()))?,
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

    fn persona_character() -> Character {
        let mut character =
            Character::new(Uuid::new_v4(), "沈知微".into(), CharacterRole::Protagonist);
        character.aliases = vec!["沈姑娘".into()];
        character.description = Some("她在第二章继承王位。".into());
        character.personality = Some("冷静".into());
        character.background = Some("失落王族".into());
        character.speaking_style = Some("言简意赅".into());
        character.appearance = Some("银色长发".into());
        character.avatar_url = Some("https://example.invalid/avatar.png".into());
        character.avatar_status = AvatarStatus::Ready;
        character.system_prompt = Some("never-public".into());
        character.first_appearance_chapter = Some(1);
        character
    }

    fn ready_novel(novel_id: Uuid, total_chapters: i32) -> Novel {
        let mut novel = Novel::create(Uuid::new_v4(), "故事".into(), None);
        novel.id = novel_id;
        novel.mark_ready(total_chapters, "世界".into(), "奇幻".into());
        novel
    }

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
    fn partial_character_json_contains_only_source_proven_identity() {
        let character = persona_character();
        let novel = ready_novel(character.novel_id, 2);

        let partial = progress_bound_character(&character, &novel, 1, true).unwrap();
        assert_eq!(
            serde_json::to_value(partial).unwrap(),
            serde_json::json!({
                "id": character.id,
                "novel_id": character.novel_id,
                "name": "沈知微",
                "first_appearance_chapter": 1,
            })
        );
    }

    #[test]
    fn full_character_json_restores_public_persona_but_never_system_prompt() {
        let character = persona_character();
        let novel = ready_novel(character.novel_id, 2);

        let full = progress_bound_character(&character, &novel, 2, false).unwrap();
        let json = serde_json::to_value(full).unwrap();

        assert_eq!(json["aliases"], serde_json::json!(["沈姑娘"]));
        assert_eq!(json["role"], "protagonist");
        assert_eq!(json["description"], "她在第二章继承王位。");
        assert_eq!(json["avatar_status"], "ready");
        assert_eq!(json["persona_source_chapter_high_water"], 2);
        assert!(json.get("system_prompt").is_none());
        assert!(json.get("created_at").is_some());
        assert!(json.get("updated_at").is_some());
    }

    #[test]
    fn full_persona_requires_ready_final_progress_and_rewind_redacts_again() {
        let character = persona_character();
        let mut novel = Novel::create(Uuid::new_v4(), "故事".into(), None);
        novel.id = character.novel_id;
        novel.start_parsing();
        novel.record_enrichment(2, "世界".into(), "奇幻".into());

        assert!(!persona_is_complete(&novel, 2));
        let parsing_final = progress_bound_character(&character, &novel, 2, true).unwrap();
        assert!(serde_json::to_value(parsing_final)
            .unwrap()
            .get("role")
            .is_none());

        novel.mark_ready(2, "世界".into(), "奇幻".into());
        assert!(!persona_is_complete(&novel, 1));
        assert!(persona_is_complete(&novel, 2));
        let full =
            serde_json::to_value(progress_bound_character(&character, &novel, 2, true).unwrap())
                .unwrap();
        assert!(full.get("role").is_some());

        let rewound =
            serde_json::to_value(progress_bound_character(&character, &novel, 1, true).unwrap())
                .unwrap();
        assert!(rewound.get("role").is_none());
        assert!(rewound.get("persona_source_chapter_high_water").is_none());
    }

    #[test]
    fn partial_character_requires_canonical_name_source_proof() {
        let character = persona_character();
        let novel = ready_novel(character.novel_id, 2);

        assert!(progress_bound_character(&character, &novel, 1, false).is_none());

        let characters = vec![character.clone()];
        let known_names = known_character_names(&characters);
        let alias_only = HashMap::from([(
            1,
            Chapter::new(character.novel_id, 1, None, "沈姑娘在门外等候。".into()),
        )]);
        assert!(!canonical_name_is_source_proven(
            &character,
            &known_names,
            &alias_only
        ));
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

#[cfg(test)]
mod reading_progress_handler_tests {
    use super::*;
    use std::collections::VecDeque;

    #[derive(Clone, Default)]
    struct CallLog(Arc<Mutex<Vec<String>>>);

    impl CallLog {
        fn push(&self, call: impl Into<String>) {
            self.0.lock().unwrap().push(call.into());
        }

        fn clear(&self) {
            self.0.lock().unwrap().clear();
        }

        fn assert_eq(&self, expected: &[&str]) {
            let calls = self.0.lock().unwrap();
            assert_eq!(
                calls.iter().map(String::as_str).collect::<Vec<_>>(),
                expected
            );
        }
    }

    struct TestNovelRepository {
        novel: Novel,
        readers: HashSet<Uuid>,
        calls: CallLog,
    }

    #[async_trait::async_trait]
    impl NovelRepository for TestNovelRepository {
        async fn create_import(&self, _novel: &Novel, _chapters: &[Chapter]) -> Result<()> {
            unreachable!("unused test repository method")
        }

        async fn create_import_batch(&self, _imports: &[(Novel, Vec<Chapter>)]) -> Result<()> {
            unreachable!("unused test repository method")
        }

        async fn create_source_import(&self, _novel: &Novel) -> Result<()> {
            unreachable!("unused test repository method")
        }

        async fn claim_import(
            &self,
            _novel_id: Uuid,
            _user_id: Uuid,
        ) -> Result<Option<ImportClaim>> {
            unreachable!("unused test repository method")
        }

        async fn recoverable_imports(
            &self,
            _limit: i64,
        ) -> Result<Vec<crate::domain::repositories::RecoverableImport>> {
            unreachable!("unused test repository method")
        }

        async fn renew_import(&self, _novel_id: Uuid, _attempt: i64) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn replace_import_chapters(
            &self,
            _novel_id: Uuid,
            _attempt: i64,
            _chapters: &[Chapter],
        ) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn record_import_enrichment(
            &self,
            _novel_id: Uuid,
            _attempt: i64,
            _total_chapters: i32,
            _world_summary: &str,
            _genre: &str,
        ) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn complete_import(&self, _novel_id: Uuid, _attempt: i64) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn fail_import(
            &self,
            _novel_id: Uuid,
            _attempt: i64,
            _failure_code: &str,
            _public_error: &str,
        ) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn find_by_id(&self, _id: Uuid) -> Result<Option<Novel>> {
            unreachable!("unused test repository method")
        }

        async fn find_for_user(&self, user_id: Uuid, novel_id: Uuid) -> Result<Option<Novel>> {
            self.calls.push("novel");
            Ok(
                (self.readers.contains(&user_id) && self.novel.id == novel_id)
                    .then(|| self.novel.clone()),
            )
        }

        async fn find_by_user(&self, _user_id: Uuid) -> Result<Vec<Novel>> {
            unreachable!("unused test repository method")
        }

        async fn find_available_to_user(&self, _user_id: Uuid) -> Result<Vec<Novel>> {
            unreachable!("unused test repository method")
        }

        async fn attach_to_user(
            &self,
            _user_id: Uuid,
            _novel_id: Uuid,
            _deviation_mode: DeviationMode,
        ) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn detach_from_user(&self, _user_id: Uuid, _novel_id: Uuid) -> Result<bool> {
            unreachable!("unused test repository method")
        }
    }

    struct TestChapterRepository {
        chapters: HashMap<i32, Chapter>,
        calls: CallLog,
    }

    #[async_trait::async_trait]
    impl ChapterRepository for TestChapterRepository {
        async fn replace_import_nodes(
            &self,
            _novel_id: Uuid,
            _attempt: i64,
            _nodes: &[(i32, String)],
        ) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn find_by_novel(&self, _novel_id: Uuid) -> Result<Vec<Chapter>> {
            unreachable!("unused test repository method")
        }

        async fn find_by_number(&self, novel_id: Uuid, number: i32) -> Result<Option<Chapter>> {
            self.calls.push(format!("chapter:{number}"));
            Ok(self
                .chapters
                .get(&number)
                .filter(|chapter| chapter.novel_id == novel_id)
                .cloned())
        }

        async fn search_lore(
            &self,
            _novel_id: Uuid,
            _max_chapter: i32,
            _query: &str,
            _limit: usize,
        ) -> Result<Vec<LoreExcerpt>> {
            unreachable!("unused test repository method")
        }
    }

    struct TestCharacterRepository {
        novel_id: Uuid,
        characters: Vec<Character>,
        calls: CallLog,
    }

    #[async_trait::async_trait]
    impl CharacterRepository for TestCharacterRepository {
        async fn replace_import(
            &self,
            _novel_id: Uuid,
            _attempt: i64,
            _characters: &[Character],
            _relationships: &[CharacterRelationshipRecord],
        ) -> Result<bool> {
            unreachable!("unused test repository method")
        }

        async fn find_by_novel(&self, novel_id: Uuid) -> Result<Vec<Character>> {
            self.calls.push("characters");
            Ok(if novel_id == self.novel_id {
                self.characters.clone()
            } else {
                Vec::new()
            })
        }

        async fn find_by_id(&self, id: Uuid) -> Result<Option<Character>> {
            self.calls.push("character");
            Ok(self
                .characters
                .iter()
                .find(|character| character.id == id)
                .cloned())
        }

        async fn set_avatar(&self, _character_id: Uuid, _avatar_url: &str) -> Result<()> {
            unreachable!("unused test repository method")
        }

        async fn find_relationships(
            &self,
            _novel_id: Uuid,
        ) -> Result<Vec<CharacterRelationshipRecord>> {
            unreachable!("unused test repository method")
        }
    }

    struct TestProgressRepository {
        scripts: Mutex<HashMap<Uuid, VecDeque<ReadingProgressRecord>>>,
        calls: CallLog,
    }

    impl TestProgressRepository {
        fn set_script(&self, user_id: Uuid, records: Vec<ReadingProgressRecord>) {
            self.scripts.lock().unwrap().insert(user_id, records.into());
        }
    }

    #[async_trait::async_trait]
    impl ReadingProgressRepository for TestProgressRepository {
        async fn get_or_create(
            &self,
            user_id: Uuid,
            novel_id: Uuid,
            _deviation_mode: &str,
        ) -> Result<ReadingProgressRecord> {
            let mut scripts = self.scripts.lock().unwrap();
            let records = scripts
                .get_mut(&user_id)
                .ok_or_else(|| anyhow::anyhow!("missing progress script"))?;
            let record = if records.len() > 1 {
                records.pop_front()
            } else {
                records.front().cloned()
            }
            .ok_or_else(|| anyhow::anyhow!("empty progress script"))?;
            anyhow::ensure!(record.novel_id == novel_id, "progress novel mismatch");
            self.calls
                .push(format!("progress:{}", record.current_chapter));
            Ok(record)
        }

        async fn update_chapter(
            &self,
            _user_id: Uuid,
            _novel_id: Uuid,
            _chapter: i32,
        ) -> Result<()> {
            unreachable!("unused test repository method")
        }

        async fn set_identity(
            &self,
            _user_id: Uuid,
            _novel_id: Uuid,
            _identity_type: &str,
            _identity_name: Option<&str>,
            _character_id: Option<Uuid>,
        ) -> Result<()> {
            unreachable!("unused test repository method")
        }
    }

    fn persona_character(novel_id: Uuid) -> Character {
        let mut character = Character::new(novel_id, "沈知微".into(), CharacterRole::Protagonist);
        character.aliases = vec!["沈姑娘".into()];
        character.description = Some("她在第二章继承王位。".into());
        character.personality = Some("冷静".into());
        character.background = Some("失落王族".into());
        character.speaking_style = Some("言简意赅".into());
        character.appearance = Some("银色长发".into());
        character.avatar_url = Some("https://example.invalid/avatar.png".into());
        character.avatar_status = AvatarStatus::Ready;
        character.system_prompt = Some("never-public".into());
        character.first_appearance_chapter = Some(1);
        character
    }

    fn progress(
        user_id: Uuid,
        novel_id: Uuid,
        current_chapter: i32,
        character: Option<&Character>,
    ) -> ReadingProgressRecord {
        let now = Utc::now();
        ReadingProgressRecord {
            id: Uuid::new_v4(),
            user_id,
            novel_id,
            current_chapter,
            reader_identity: character.map(|character| character.name.clone()),
            reader_identity_type: if character.is_some() {
                "character".into()
            } else {
                "self".into()
            },
            reader_character_id: character.map(|character| character.id),
            deviation_mode: "canon".into(),
            last_read_at: now,
            created_at: now,
        }
    }

    fn handler(
        novel: Novel,
        readers: &[Uuid],
        characters: Vec<Character>,
        chapters: Vec<Chapter>,
        progress_records: Vec<ReadingProgressRecord>,
    ) -> (
        ReadingProgressHandler,
        CallLog,
        Arc<TestCharacterRepository>,
        Arc<TestProgressRepository>,
    ) {
        let calls = CallLog::default();
        let novel_repo = Arc::new(TestNovelRepository {
            novel: novel.clone(),
            readers: readers.iter().copied().collect(),
            calls: calls.clone(),
        });
        let chapter_repo = Arc::new(TestChapterRepository {
            chapters: chapters
                .into_iter()
                .map(|chapter| (chapter.chapter_number, chapter))
                .collect(),
            calls: calls.clone(),
        });
        let character_repo = Arc::new(TestCharacterRepository {
            novel_id: novel.id,
            characters,
            calls: calls.clone(),
        });
        let progress_repo = Arc::new(TestProgressRepository {
            scripts: Mutex::new(progress_records.into_iter().fold(
                HashMap::<Uuid, VecDeque<ReadingProgressRecord>>::new(),
                |mut scripts, record| {
                    scripts.entry(record.user_id).or_default().push_back(record);
                    scripts
                },
            )),
            calls: calls.clone(),
        });
        (
            ReadingProgressHandler {
                novel_repo,
                chapter_repo,
                character_repo: character_repo.clone(),
                progress_repo: progress_repo.clone(),
            },
            calls,
            character_repo,
            progress_repo,
        )
    }

    fn ready_novel(novel_id: Uuid, total_chapters: i32) -> Novel {
        let mut novel = Novel::create(Uuid::new_v4(), "故事".into(), None);
        novel.id = novel_id;
        novel.mark_ready(total_chapters, "世界".into(), "奇幻".into());
        novel
    }

    #[tokio::test]
    async fn list_and_detail_are_progress_bound_per_user_without_mutating_characters() {
        let novel_id = Uuid::new_v4();
        let character = persona_character(novel_id);
        let mut future = Character::new(novel_id, "顾远".into(), CharacterRole::Supporting);
        future.description = Some("第二章才揭示的角色。".into());
        future.first_appearance_chapter = Some(2);
        let novel = ready_novel(novel_id, 2);
        let full_user = Uuid::new_v4();
        let partial_user = Uuid::new_v4();
        let (handler, calls, character_repo, _) = handler(
            novel,
            &[full_user, partial_user],
            vec![character.clone(), future],
            vec![
                Chapter::new(novel_id, 1, None, "沈知微在庭院里。".into()),
                Chapter::new(novel_id, 2, None, "顾远走进大厅。".into()),
            ],
            vec![
                progress(full_user, novel_id, 2, None),
                progress(partial_user, novel_id, 1, None),
            ],
        );

        let full_list = handler
            .list_available_characters(full_user, novel_id)
            .await
            .unwrap();
        calls.assert_eq(&[
            "novel",
            "characters",
            "progress:2",
            "chapter:2",
            "progress:2",
        ]);
        let full_json = serde_json::to_value(
            full_list
                .iter()
                .find(|candidate| candidate.id == character.id)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(full_json["role"], "protagonist");
        assert_eq!(full_json["persona_source_chapter_high_water"], 2);
        assert!(full_json.get("system_prompt").is_none());

        calls.clear();
        let partial_list = handler
            .list_available_characters(partial_user, novel_id)
            .await
            .unwrap();
        calls.assert_eq(&[
            "novel",
            "characters",
            "progress:1",
            "chapter:1",
            "chapter:1",
            "progress:1",
        ]);
        assert_eq!(partial_list.len(), 1);
        let partial_json = serde_json::to_value(&partial_list[0]).unwrap();
        assert_eq!(
            partial_json,
            serde_json::json!({
                "id": character.id,
                "novel_id": novel_id,
                "name": "沈知微",
                "first_appearance_chapter": 1,
            })
        );

        calls.clear();
        let partial_detail = handler
            .get_available_character(partial_user, character.id)
            .await
            .unwrap();
        calls.assert_eq(&[
            "character",
            "novel",
            "characters",
            "progress:1",
            "chapter:1",
            "chapter:1",
            "progress:1",
        ]);
        assert_eq!(serde_json::to_value(partial_detail).unwrap(), partial_json);

        calls.clear();
        let full_detail = handler
            .get_available_character(full_user, character.id)
            .await
            .unwrap();
        calls.assert_eq(&[
            "character",
            "novel",
            "characters",
            "progress:2",
            "chapter:2",
            "progress:2",
        ]);
        assert_eq!(serde_json::to_value(full_detail).unwrap(), full_json);
        assert_eq!(
            character_repo.characters[0].system_prompt.as_deref(),
            Some("never-public")
        );
    }

    #[tokio::test]
    async fn complete_to_rewind_race_never_returns_a_full_list_or_detail() {
        let novel_id = Uuid::new_v4();
        let character = persona_character(novel_id);
        let novel = ready_novel(novel_id, 2);
        let user_id = Uuid::new_v4();
        let full = progress(user_id, novel_id, 2, None);
        let rewound = progress(user_id, novel_id, 1, None);
        let (handler, calls, _, progress_repo) = handler(
            novel,
            &[user_id],
            vec![character.clone()],
            vec![
                Chapter::new(novel_id, 1, None, "沈知微在庭院里。".into()),
                Chapter::new(novel_id, 2, None, "第二章。".into()),
            ],
            vec![full.clone(), rewound.clone()],
        );

        assert!(matches!(
            handler.list_available_characters(user_id, novel_id).await,
            Err(ReadingProgressError::Internal(_))
        ));
        calls.assert_eq(&[
            "novel",
            "characters",
            "progress:2",
            "chapter:2",
            "progress:1",
        ]);

        progress_repo.set_script(user_id, vec![full, rewound]);
        calls.clear();
        assert!(matches!(
            handler.get_available_character(user_id, character.id).await,
            Err(ReadingProgressError::Internal(_))
        ));
        calls.assert_eq(&[
            "character",
            "novel",
            "characters",
            "progress:2",
            "chapter:2",
            "progress:1",
        ]);
    }

    #[tokio::test]
    async fn stable_alias_only_identity_after_rewind_is_typed_unavailable() {
        let novel_id = Uuid::new_v4();
        let character = persona_character(novel_id);
        let novel = ready_novel(novel_id, 2);
        let user_id = Uuid::new_v4();
        let (handler, calls, _, _) = handler(
            novel,
            &[user_id],
            vec![character.clone()],
            vec![
                Chapter::new(novel_id, 1, None, "沈姑娘在庭院里。".into()),
                Chapter::new(novel_id, 2, None, "第二章。".into()),
            ],
            vec![progress(user_id, novel_id, 1, Some(&character))],
        );

        assert!(matches!(
            handler.get(user_id, novel_id).await,
            Err(ReadingProgressError::IdentityUnavailable)
        ));
        calls.assert_eq(&[
            "novel",
            "progress:1",
            "chapter:1",
            "characters",
            "chapter:1",
            "progress:1",
        ]);
    }
}
