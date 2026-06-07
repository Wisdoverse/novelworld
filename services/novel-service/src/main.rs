#![allow(dead_code, unused_imports)]
mod domain;
mod application;
mod infrastructure;
mod interface;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tower_http::cors::{CorsLayer, Any};

use application::handlers::NovelCommandHandler;
use domain::ports::{LlmPort, ImagePort};
use infrastructure::llm::{LlmAdapter, image::ImageClient};
use infrastructure::persistence::{
    novel_pg_repo::NovelPgRepository,
    chapter_pg_repo::ChapterPgRepository,
    character_pg_repo::CharacterPgRepository,
    pg_progress_repo::PgReadingProgressRepository,
};
use interface::http::{router, AppState};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    dotenvy::dotenv().ok();

    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await?;

    tracing::info!("Connected to PostgreSQL");

    let llm_base = Arc::new(llm_client::LlmClient::new()
        .with_openai_compatible("default",
            std::env::var("LLM_API_KEY").expect("LLM_API_KEY must be set"),
            std::env::var("LLM_API_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
        ));
    let llm_model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o".into());
    let llm = Arc::new(LlmAdapter::new(llm_base, format!("default/{}", llm_model)));

    let image_client = Arc::new(ImageClient::new(
        std::env::var("IMAGE_GEN_API_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
        std::env::var("IMAGE_GEN_API_KEY").unwrap_or_else(|_|
            std::env::var("LLM_API_KEY").expect("LLM_API_KEY must be set")
        ),
        std::env::var("IMAGE_GEN_MODEL").unwrap_or_else(|_| "dall-e-3".into()),
    ));

    let novel_repo = Arc::new(NovelPgRepository::new(pool.clone()));
    let chapter_repo = Arc::new(ChapterPgRepository::new(pool.clone()));
    let character_repo = Arc::new(CharacterPgRepository::new(pool.clone()));
    let progress_repo = Arc::new(PgReadingProgressRepository::new(pool.clone()));

    let llm: Arc<dyn LlmPort> = llm;
    let image_client: Arc<dyn ImagePort> = image_client;

    let handler = Arc::new(NovelCommandHandler {
        novel_repo: novel_repo.clone(),
        chapter_repo: chapter_repo.clone(),
        character_repo: character_repo.clone(),
        llm,
        image_client,
    });

    let state = AppState {
        handler,
        novel_repo,
        chapter_repo,
        character_repo,
        progress_repo,
    };

    let app = router(state)
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any));

    let port = std::env::var("PORT").unwrap_or_else(|_| "8002".into());
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("novel-service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

async fn shutdown_signal() {
    use tokio::signal;
    let ctrl_c = signal::ctrl_c();
    #[cfg(unix)]
    let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate()).unwrap();
    #[cfg(unix)]
    tokio::select! {
        _ = ctrl_c => { tracing::info!("Received SIGINT"); }
        _ = sigterm.recv() => { tracing::info!("Received SIGTERM"); }
    }
    #[cfg(not(unix))]
    ctrl_c.await.ok();
}
