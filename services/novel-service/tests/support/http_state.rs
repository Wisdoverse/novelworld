use async_trait::async_trait;
use novel_service::{
    application::handlers::{NovelCommandHandler, ReadingProgressHandler, TranslateChapterHandler},
    domain::{
        ports::{
            DocumentTextExtractor, ImagePort, LlmPort, NovelLlmTask, PrivacyCleanupPort,
            ReadinessProbe, TextTranslator,
        },
        repositories::SourceFileDeletionRepository,
    },
    infrastructure::{
        document::EbookTextExtractor,
        persistence::{
            account_export::PgAccountExport,
            canon_story_model_pg_repo::PgCanonStoryModelRepository,
            chapter_pg_repo::ChapterPgRepository,
            chapter_translation_pg_repo::PgChapterTranslationRepository,
            character_pg_repo::CharacterPgRepository, novel_pg_repo::NovelPgRepository,
            pg_progress_repo::PgReadingProgressRepository,
            source_file_deletion_pg_repo::PgSourceFileDeletionRepository,
        },
    },
    interface::http::AppState,
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;
use uuid::Uuid;

struct NeverProvider;

#[async_trait]
impl LlmPort for NeverProvider {
    async fn chat_json(
        &self,
        _user_id: Uuid,
        _task: NovelLlmTask,
        _prompt: &str,
    ) -> anyhow::Result<String> {
        unreachable!("route must not call its provider")
    }
}

#[async_trait]
impl TextTranslator for NeverProvider {
    async fn to_simplified_chinese(&self, _user_id: Uuid, _source: &str) -> anyhow::Result<String> {
        unreachable!("route must not call its provider")
    }
}

#[async_trait]
impl ImagePort for NeverProvider {
    async fn generate(&self, _prompt: &str) -> anyhow::Result<String> {
        unreachable!("route must not call its provider")
    }
}

#[async_trait]
impl PrivacyCleanupPort for NeverProvider {
    async fn clear_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> anyhow::Result<()> {
        unreachable!("route must not call its provider")
    }

    async fn allow_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> anyhow::Result<()> {
        unreachable!("route must not call its provider")
    }
}

struct FixedProbe;

#[async_trait]
impl ReadinessProbe for FixedProbe {
    async fn is_ready(&self) -> bool {
        true
    }
}

pub fn state_with_pool(pool: sqlx::PgPool) -> AppState {
    let novel_repo = Arc::new(NovelPgRepository::new(pool.clone()));
    let chapter_repo = Arc::new(ChapterPgRepository::new(pool.clone()));
    let character_repo = Arc::new(CharacterPgRepository::new(pool.clone()));
    let canon_repo = Arc::new(PgCanonStoryModelRepository::new(pool.clone()));
    let progress_repo = Arc::new(PgReadingProgressRepository::new(pool.clone()));
    let translation_repo = Arc::new(PgChapterTranslationRepository::new(pool.clone()));
    let source_deletions: Arc<dyn SourceFileDeletionRepository> =
        Arc::new(PgSourceFileDeletionRepository::new(pool.clone()));
    let provider = Arc::new(NeverProvider);
    let document_extractor: Arc<dyn DocumentTextExtractor> = Arc::new(EbookTextExtractor);
    let handler = Arc::new(NovelCommandHandler {
        novel_repo: novel_repo.clone(),
        chapter_repo: chapter_repo.clone(),
        character_repo: character_repo.clone(),
        canon_repo: canon_repo.clone(),
        llm: provider.clone(),
        image_client: provider.clone(),
        privacy_cleanup: provider.clone(),
        source_storage: None,
        source_deletions,
        document_extractor: document_extractor.clone(),
        acceptance_permits: Arc::new(Semaphore::new(2)),
        import_permits: Arc::new(Semaphore::new(1)),
        active_import_users: Arc::new(Mutex::new(HashSet::new())),
    });
    AppState {
        series_handler: Arc::new(novel_service::application::world_series::WorldSeriesHandler {
            series_repo: Arc::new(novel_service::infrastructure::persistence::world_series_pg_repo::PgWorldSeriesRepository::new(pool.clone())),
            novel_repo: novel_repo.clone(),
            canon_repo: canon_repo.clone(),
            character_repo: character_repo.clone(),
            matcher: None,
            llm: None,
        }),
        handler,
        novel_repo: novel_repo.clone(),
        chapter_repo: chapter_repo.clone(),
        character_repo: character_repo.clone(),
        canon_repo: canon_repo.clone(),
        progress_handler: Arc::new(ReadingProgressHandler {
            novel_repo,
            chapter_repo: chapter_repo.clone(),
            character_repo,
            canon_repo,
            progress_repo,
        }),
        translation_handler: Arc::new(TranslateChapterHandler {
            chapter_repo,
            translation_repo,
            translator: provider,
            permits: Arc::new(Semaphore::new(1)),
        }),
        document_extractor,
        document_parse_permits: Arc::new(Semaphore::new(1)),
        account_export: Arc::new(PgAccountExport::new(pool)),
        internal_service_token: Arc::from("expected-token"),
        readiness: Arc::new(FixedProbe),
        source_storage_readiness: None,
        metrics: {
            static METRICS: std::sync::OnceLock<llm_client::MetricsHandle> = std::sync::OnceLock::new();
            METRICS.get_or_init(|| llm_client::install_metrics("novel-service-http-test").unwrap()).clone()
        },
    }
}
