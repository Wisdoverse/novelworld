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

use application::handlers::AgentCommandHandler;
use domain::services::memory_manager::MemoryManager;
use infrastructure::llm::LlmClient;
use infrastructure::cache::RedisCache;
use infrastructure::persistence::{
    pg_memory_repo::PgMemoryRepository,
    pg_chat_repo::PgChatRepository,
    pg_character_info_repo::PgCharacterInfoRepository,
};
use interface::http::{router, AppState};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    dotenvy::dotenv().ok();

    // PostgreSQL connection pool
    let database_url = std::env::var("DATABASE_URL")
        .expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await?;

    tracing::info!("Connected to PostgreSQL");

    // Redis connection pool
    let redis_url = std::env::var("REDIS_URL")
        .unwrap_or_else(|_| "redis://redis:6379".into());
    let redis_cfg = deadpool_redis::Config::from_url(&redis_url);
    let redis_pool = redis_cfg
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("Failed to create Redis pool");

    tracing::info!("Redis pool created");

    // LLM client
    let llm = Arc::new(LlmClient::new(
        std::env::var("LLM_API_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
        std::env::var("LLM_API_KEY").expect("LLM_API_KEY must be set"),
        std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o".into()),
    ));

    // Repositories
    let memory_repo = Arc::new(PgMemoryRepository::new(pool.clone()));
    let chat_repo = Arc::new(PgChatRepository::new(pool.clone()));
    let character_repo = Arc::new(PgCharacterInfoRepository::new(pool.clone()));

    // Redis cache
    let cache = Arc::new(RedisCache::new(redis_pool));

    // Memory manager (4-layer memory pyramid)
    let memory_manager = Arc::new(MemoryManager {
        memory_repo: memory_repo.clone(),
        chat_repo: chat_repo.clone(),
        cache: cache.clone(),
        llm: llm.clone(),
    });

    // Application handler
    let handler = Arc::new(AgentCommandHandler {
        memory_manager,
        character_repo,
        llm,
    });

    let state = AppState { handler };

    // Router
    let app = router(state)
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any));

    let port = std::env::var("PORT").unwrap_or_else(|_| "8003".into());
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("agent-service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
