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

use application::handlers::NarrativeCommandHandler;
use infrastructure::llm::LlmClient;
use infrastructure::persistence::{
    pg_narrative_repo::{PgNarrativeNodeRepository, PgUserChoiceRepository},
    pg_world_state_repo::PgWorldStateRepository,
    pg_chapter_read_repo::PgChapterReadRepository,
};
use interface::http::{router, AppState};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    dotenvy::dotenv().ok();

    // Database connection pool
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await?;

    tracing::info!("Connected to PostgreSQL");

    // LLM client
    let llm = Arc::new(LlmClient::new(
        std::env::var("LLM_API_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
        std::env::var("LLM_API_KEY").expect("LLM_API_KEY must be set"),
        std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o".into()),
    ));

    // Repositories
    let node_repo = Arc::new(PgNarrativeNodeRepository::new(pool.clone()));
    let choice_repo = Arc::new(PgUserChoiceRepository::new(pool.clone()));
    let world_state_repo = Arc::new(PgWorldStateRepository::new(pool.clone()));
    let chapter_repo = Arc::new(PgChapterReadRepository::new(pool.clone()));

    // Application handler
    let handler = Arc::new(NarrativeCommandHandler {
        node_repo,
        choice_repo,
        world_state_repo,
        chapter_repo,
        llm,
    });

    let state = AppState { handler };

    // Router with CORS
    let app = router(state)
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any));

    let port = std::env::var("PORT").unwrap_or_else(|_| "8004".into());
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("narrative-service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
