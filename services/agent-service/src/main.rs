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

use application::handlers::AgentCommandHandler;
use domain::services::memory_manager::MemoryManager;
use infrastructure::llm::LlmAdapter;
use infrastructure::cache::RedisCache;
use infrastructure::embedding::EmbeddingAdapter;
use infrastructure::http::novel_client::NovelServiceClient;
use infrastructure::persistence::{
    pg_memory_repo::PgMemoryRepository,
    pg_chat_repo::PgChatRepository,
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

    // Shared LLM client (from llm-client workspace crate)
    let api_key = std::env::var("LLM_API_KEY").unwrap_or_default();
    let api_url = std::env::var("LLM_API_URL").unwrap_or_else(|_| "https://api.openai.com".into());
    let model = std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o".into());

    let llm_base = Arc::new(
        llm_client::LlmClient::new()
            .with_openai_compatible("default", &api_key, &api_url),
    );

    // LLM adapter for chat (TextSummarizer + handler direct calls)
    let llm = Arc::new(LlmAdapter::new(llm_base.clone(), format!("default/{}", model)));

    // Repositories
    let memory_repo = Arc::new(PgMemoryRepository::new(pool.clone()));
    let chat_repo = Arc::new(PgChatRepository::new(pool.clone()));

    // Character info via HTTP to novel-service (replaces direct DB coupling)
    let novel_service_url = std::env::var("NOVEL_SERVICE_URL")
        .unwrap_or_else(|_| "http://novel-service:8002".into());
    let character_repo = Arc::new(NovelServiceClient::new(novel_service_url));

    // Redis cache
    let cache = Arc::new(RedisCache::new(redis_pool));

    // Embedding adapter (reuses shared LLM client for embedding API)
    let embed_api_key = std::env::var("EMBEDDING_API_KEY").unwrap_or_else(|_| api_key.clone());
    let embed_api_url = std::env::var("EMBEDDING_API_URL").unwrap_or_else(|_| api_url.clone());
    let embed_model = std::env::var("EMBEDDING_MODEL").unwrap_or_else(|_| "text-embedding-3-small".into());

    let embed_base = Arc::new(
        llm_client::LlmClient::new()
            .with_openai_compatible("embed", &embed_api_key, &embed_api_url),
    );
    let embedding: Arc<dyn domain::ports::EmbeddingGenerator> =
        Arc::new(EmbeddingAdapter::new(embed_base, format!("embed/{}", embed_model)));

    // Memory manager (4-layer memory pyramid)
    let memory_manager = Arc::new(MemoryManager {
        memory_repo: memory_repo.clone(),
        chat_repo: chat_repo.clone(),
        cache: cache.clone() as Arc<dyn domain::ports::MessageCache>,
        llm: llm.clone() as Arc<dyn domain::ports::TextSummarizer>,
        embedding,
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
