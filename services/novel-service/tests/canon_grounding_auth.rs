use async_trait::async_trait;
use axum::{
    body::Body,
    http::{header::CACHE_CONTROL, HeaderValue, Request, StatusCode},
};
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
    interface::http::{router, AppState},
};
use sqlx::postgres::PgPoolOptions;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;
use tower::ServiceExt;
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
        unreachable!("unauthorized route must not call its provider")
    }
}

#[async_trait]
impl TextTranslator for NeverProvider {
    async fn to_simplified_chinese(&self, _user_id: Uuid, _source: &str) -> anyhow::Result<String> {
        unreachable!("unauthorized route must not call its provider")
    }
}

#[async_trait]
impl ImagePort for NeverProvider {
    async fn generate(&self, _prompt: &str) -> anyhow::Result<String> {
        unreachable!("unauthorized route must not call its provider")
    }
}

#[async_trait]
impl PrivacyCleanupPort for NeverProvider {
    async fn clear_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> anyhow::Result<()> {
        unreachable!("unauthorized route must not call its provider")
    }

    async fn allow_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> anyhow::Result<()> {
        unreachable!("unauthorized route must not call its provider")
    }
}

struct FixedProbe;

#[async_trait]
impl ReadinessProbe for FixedProbe {
    async fn is_ready(&self) -> bool {
        true
    }
}

fn auth_test_state() -> AppState {
    let pool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(10))
        .connect_lazy("postgres://unused:unused@127.0.0.1:1/unused")
        .unwrap();
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
            matcher: None,
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

#[tokio::test]
async fn canon_grounding_route_rejects_missing_and_wrong_internal_tokens() {
    let app = router(auth_test_state());
    let uri = format!(
        "/internal/novels/{}/characters/{}/grounding-v1/3",
        Uuid::new_v4(),
        Uuid::new_v4()
    );
    for token in [None, Some("wrong-token")] {
        let mut request = Request::builder()
            .uri(&uri)
            .header("X-User-Id", Uuid::new_v4().to_string());
        if let Some(token) = token {
            request = request.header("X-Internal-Service-Token", token);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers().get(CACHE_CONTROL),
            Some(&HeaderValue::from_static("private, no-store"))
        );
    }
}

#[tokio::test]
async fn series_routes_reject_missing_principal_before_database_or_provider_work() {
    let app = router(auth_test_state());
    let novel = Uuid::new_v4();
    let create = serde_json::json!({"name":"同一世界", "background":"用户确认的共同背景", "source_novel_id":novel});
    for (method, uri, body) in [
        ("GET", "/novels/world-series".into(), None),
        (
            "POST",
            "/novels/world-series".into(),
            Some(create.to_string()),
        ),
        ("GET", format!("/novels/{novel}/world-series"), None),
        (
            "PUT",
            format!("/novels/{novel}/world-series"),
            Some(r#"{"series_id":null}"#.into()),
        ),
        (
            "POST",
            format!("/novels/{novel}/world-series/suggestion"),
            None,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.unwrap_or_default()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(
            response.headers()[CACHE_CONTROL],
            HeaderValue::from_static("private, no-store")
        );
    }
}

#[tokio::test]
async fn direct_series_generation_is_rejected_without_a_database_claim() {
    let app = router(auth_test_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/internal/novels/{}/game-rules?prompt_version=series-game-rules-v1",
                    Uuid::new_v4()
                ))
                .header("X-User-Id", Uuid::new_v4().to_string())
                .header("X-Internal-Service-Token", "expected-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = axum::body::to_bytes(response.into_body(), 16 * 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(body["error"]["code"], "unsupported_game_rule_version");
}
