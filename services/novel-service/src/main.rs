#![allow(dead_code, unused_imports)]
use anyhow::Result;
use axum::{extract::Request, middleware, middleware::Next, response::Response};
use sqlx::postgres::PgPoolOptions;
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};
use tokio::sync::Semaphore;
use tower_http::cors::{Any, CorsLayer};
use tracing::Instrument;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use novel_service::{
    application::{
        handlers::{NovelCommandHandler, ReadingProgressHandler, TranslateChapterHandler},
        source_file_cleanup::SourceFileCleanupWorker,
    },
    domain::ports::{
        AccountExportPort, DocumentTextExtractor, ImagePort, LlmPort, PrivacyCleanupPort,
        ReadinessProbe, SourceFileStorage, TextTranslator,
    },
    infrastructure::{
        document::EbookTextExtractor,
        llm::{image::ImageClient, LlmAdapter},
        object_storage::{S3SourceFileStorage, S3StorageConfig},
        persistence::{
            account_export::PgAccountExport,
            canon_story_model_pg_repo::PgCanonStoryModelRepository,
            chapter_pg_repo::ChapterPgRepository,
            chapter_translation_pg_repo::PgChapterTranslationRepository,
            character_pg_repo::CharacterPgRepository, novel_pg_repo::NovelPgRepository,
            pg_progress_repo::PgReadingProgressRepository,
            source_file_deletion_pg_repo::PgSourceFileDeletionRepository, PgReadinessProbe,
        },
        privacy::AgentPrivacyClient,
    },
    interface::http::{router, AppState},
};

/// SPEC 14.1: wrap every request in a span carrying the service name and the
/// propagated X-Trace-Id.
async fn trace_middleware(request: Request, next: Next) -> Response {
    let trace_id = request
        .headers()
        .get("x-trace-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let span = tracing::info_span!("service", service = "novel-service", trace_id = %trace_id);
    async {
        let response = next.run(request).await;
        // SPEC 14.1: one request-scoped entry per request, so the log contract
        // holds even when no other service event fires while handling it.
        tracing::info!("request completed");
        response
    }
    .instrument(span)
    .await
}

#[tokio::main]
async fn main() -> Result<()> {
    run_body().await
}

async fn run_body() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(format!(
            "{},reqwest=off,tower_http=off",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())
        )))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let service_span = tracing::info_span!("service", service = "novel-service", trace_id = "");
    async move {
        // SPEC 14.1: every log entry carries the service name and a trace id
        // (empty outside a request, the propagated X-Trace-Id while handling one).

        dotenvy::dotenv().ok();
        let metrics = llm_client::install_metrics("novel-service")?;
        let internal_service_token =
            std::env::var("INTERNAL_SERVICE_TOKEN").expect("INTERNAL_SERVICE_TOKEN must be set");
        if internal_service_token.len() < 32 {
            anyhow::bail!("INTERNAL_SERVICE_TOKEN must be at least 32 characters");
        }

        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(&database_url)
            .await?;

        tracing::info!("Connected to PostgreSQL");

        let llm_adapter = Arc::new(LlmAdapter::new(Arc::new(
            llm_client::RuntimeLlmClient::from_env()?,
        )));

        let image_client = Arc::new(ImageClient::new(
            std::env::var("IMAGE_GEN_API_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
            std::env::var("IMAGE_GEN_API_KEY")
                .unwrap_or_else(|_| std::env::var("LLM_API_KEY").unwrap_or_default()),
            std::env::var("IMAGE_GEN_MODEL").unwrap_or_else(|_| "dall-e-3".into()),
        ));

        let novel_repo = Arc::new(NovelPgRepository::new(pool.clone()));
        let chapter_repo = Arc::new(ChapterPgRepository::new(pool.clone()));
        let translation_repo = Arc::new(PgChapterTranslationRepository::new(pool.clone()));
        let character_repo = Arc::new(CharacterPgRepository::new(pool.clone()));
        let canon_repo = Arc::new(PgCanonStoryModelRepository::new(pool.clone()));
        let progress_repo = Arc::new(PgReadingProgressRepository::new(pool.clone()));
        let account_export: Arc<dyn AccountExportPort> =
            Arc::new(PgAccountExport::new(pool.clone()));
        let source_deletions_impl = Arc::new(PgSourceFileDeletionRepository::new(pool.clone()));
        let (source_storage, source_storage_readiness) = match S3StorageConfig::from_env()? {
            Some(config) => {
                let storage = Arc::new(S3SourceFileStorage::new(config).await?);
                let port: Arc<dyn SourceFileStorage> = storage.clone();
                let readiness: Arc<dyn ReadinessProbe> = storage;
                (Some(port), Some(readiness))
            }
            None => {
                if source_deletions_impl.storage_required().await? {
                    anyhow::bail!("S3_ENABLED must remain true while stored source files exist");
                }
                (None, None)
            }
        };
        let source_deletions: Arc<
            dyn novel_service::domain::repositories::SourceFileDeletionRepository,
        > = source_deletions_impl;
        if let Some(storage) = source_storage.clone() {
            SourceFileCleanupWorker::new(storage, source_deletions.clone()).spawn();
        }

        let translator: Arc<dyn TextTranslator> = llm_adapter.clone();
        let llm: Arc<dyn LlmPort> = llm_adapter;
        let image_client: Arc<dyn ImagePort> = image_client;
        let document_extractor: Arc<dyn DocumentTextExtractor> = Arc::new(EbookTextExtractor);
        let privacy_cleanup: Arc<dyn PrivacyCleanupPort> = Arc::new(AgentPrivacyClient::new(
            std::env::var("AGENT_SERVICE_URL")
                .unwrap_or_else(|_| "http://agent-service:8003".into()),
            internal_service_token.clone(),
        )?);

        let handler = Arc::new(NovelCommandHandler {
            novel_repo: novel_repo.clone(),
            chapter_repo: chapter_repo.clone(),
            character_repo: character_repo.clone(),
            canon_repo: canon_repo.clone(),
            llm,
            image_client,
            privacy_cleanup,
            source_storage,
            source_deletions,
            document_extractor: document_extractor.clone(),
            // ponytail: process-local admission matches the single service replica;
            // replace with a durable queue before horizontally scaling imports.
            import_permits: Arc::new(Semaphore::new(2)),
            active_import_users: Arc::new(Mutex::new(HashSet::new())),
        });
        let _import_recovery = handler.spawn_import_recovery();
        let progress_handler = Arc::new(ReadingProgressHandler {
            novel_repo: novel_repo.clone(),
            chapter_repo: chapter_repo.clone(),
            character_repo: character_repo.clone(),
            progress_repo,
        });
        let translation_handler = Arc::new(TranslateChapterHandler {
            chapter_repo: chapter_repo.clone(),
            translation_repo,
            translator,
            // ponytail: process-local admission matches the single-node runtime contract.
            permits: Arc::new(Semaphore::new(2)),
        });

        let state = AppState {
            handler,
            novel_repo,
            chapter_repo,
            character_repo,
            canon_repo,
            progress_handler,
            translation_handler,
            document_extractor,
            document_parse_permits: Arc::new(Semaphore::new(2)),
            account_export,
            internal_service_token: internal_service_token.into(),
            readiness: Arc::new(PgReadinessProbe::new(pool)),
            source_storage_readiness,
            metrics,
        };

        let app = router(state)
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .layer(middleware::from_fn(trace_middleware));

        let port = std::env::var("PORT").unwrap_or_else(|_| "8002".into());
        let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0".into());
        let addr = format!("{}:{}", bind_addr, port);
        tracing::info!("novel-service listening on {}", addr);

        let listener = tokio::net::TcpListener::bind(&addr).await?;
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await?;

        Ok(())
    }
    .instrument(service_span)
    .await
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = signal::ctrl_c();
    #[cfg(unix)]
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate()).unwrap();
    #[cfg(unix)]
    tokio::select! {
        _ = ctrl_c => { tracing::info!("Received SIGINT"); }
        _ = sigterm.recv() => { tracing::info!(service = "novel-service", trace_id = "", "Received SIGTERM"); }
    }
    #[cfg(not(unix))]
    ctrl_c.await.ok();
}
