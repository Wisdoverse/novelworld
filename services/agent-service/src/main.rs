#![allow(dead_code, unused_imports)]
use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use agent_service::{
    application::handlers::AgentCommandHandler,
    domain::{self, ports::AccountExportPort, services::memory_manager::MemoryManager},
    infrastructure::{
        cache::{RedisCache, RedisReadinessProbe},
        embedding::EmbeddingAdapter,
        http::novel_client::NovelServiceClient,
        llm::LlmAdapter,
        persistence::{
            account_export::PgAccountExport, pg_chat_repo::PgChatRepository,
            pg_memory_repo::PgMemoryRepository, PgReadinessProbe,
        },
    },
    interface::http::{router, AppState},
};

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
    let metrics = llm_client::install_metrics("agent-service")?;
    let internal_service_token =
        std::env::var("INTERNAL_SERVICE_TOKEN").expect("INTERNAL_SERVICE_TOKEN must be set");
    if internal_service_token.len() < 32 {
        anyhow::bail!("INTERNAL_SERVICE_TOKEN must be at least 32 characters");
    }

    // PostgreSQL connection pool
    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(20)
        .connect(&database_url)
        .await?;

    tracing::info!("Connected to PostgreSQL");

    // Redis connection pool
    let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".into());
    let redis_cfg = deadpool_redis::Config::from_url(&redis_url);
    let redis_pool = redis_cfg
        .create_pool(Some(deadpool_redis::Runtime::Tokio1))
        .expect("Failed to create Redis pool");

    tracing::info!("Redis pool created");

    // Shared LLM client (from llm-client workspace crate)
    let api_key = std::env::var("LLM_API_KEY").unwrap_or_default();
    let api_url = std::env::var("LLM_API_URL").unwrap_or_else(|_| "https://api.openai.com".into());
    let llm_base = Arc::new(llm_client::RuntimeLlmClient::from_env()?);

    // LLM adapter for chat (TextSummarizer + handler direct calls)
    let llm = Arc::new(LlmAdapter::new(llm_base));

    // Repositories
    let memory_repo = Arc::new(PgMemoryRepository::new(pool.clone()));
    let chat_repo = Arc::new(PgChatRepository::new(pool.clone()));
    let account_export: Arc<dyn AccountExportPort> = Arc::new(PgAccountExport::new(pool.clone()));

    // Character info via HTTP to novel-service (replaces direct DB coupling)
    let novel_service_url =
        std::env::var("NOVEL_SERVICE_URL").unwrap_or_else(|_| "http://novel-service:8002".into());
    let novel_client = Arc::new(NovelServiceClient::new(novel_service_url));
    let character_repo: Arc<dyn domain::repositories::CharacterInfoRepository> =
        novel_client.clone();
    let reading_context: Arc<dyn domain::ports::ReadingContextPort> = novel_client.clone();
    let lore_context: Arc<dyn domain::ports::LoreContextPort> = novel_client.clone();
    let novel_readiness: Arc<dyn domain::ports::ReadinessProbe> = novel_client;

    // Redis cache
    let cache = Arc::new(RedisCache::new(redis_pool.clone()));

    // Embedding adapter — auto-select model based on provider
    let embed_api_key = std::env::var("EMBEDDING_API_KEY").unwrap_or_else(|_| api_key.clone());
    let embed_api_url = std::env::var("EMBEDDING_API_URL").unwrap_or_else(|_| api_url.clone());
    let embed_model = std::env::var("EMBEDDING_MODEL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| auto_detect_embedding_model(&embed_api_url));

    let embedding: Arc<dyn domain::ports::EmbeddingGenerator> =
        if embed_api_key.is_empty() && !embed_api_url.contains("localhost") {
            tracing::info!(
                "No embedding API key — semantic search disabled, using other memory layers"
            );
            Arc::new(NoopEmbeddingGenerator)
        } else {
            let embed_base = Arc::new(llm_client::LlmClient::new().with_openai_compatible(
                "embed",
                &embed_api_key,
                &embed_api_url,
            ));
            tracing::info!("Embedding model: {}", embed_model);
            Arc::new(EmbeddingAdapter::new(
                embed_base,
                format!("embed/{}", embed_model),
            ))
        };

    // Memory manager (4-layer memory pyramid)
    let memory_manager = Arc::new(MemoryManager {
        memory_repo: memory_repo.clone(),
        chat_repo: chat_repo.clone(),
        cache: cache.clone() as Arc<dyn domain::ports::MessageCache>,
        llm: llm.clone() as Arc<dyn domain::ports::TextSummarizer>,
        embedding,
    });

    // Application handler
    let chat_llm: Arc<dyn domain::ports::ChatCompletion> = llm.clone();
    let handler = Arc::new(AgentCommandHandler {
        memory_manager,
        character_repo,
        reading_context,
        lore_context,
        llm: chat_llm,
    });

    let state = AppState {
        handler,
        postgres_readiness: Arc::new(PgReadinessProbe::new(pool)),
        redis_readiness: Arc::new(RedisReadinessProbe::new(redis_pool)),
        novel_readiness,
        account_export,
        internal_service_token: internal_service_token.into(),
        metrics,
    };

    // Router
    let app = router(state).layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );

    let port = std::env::var("PORT").unwrap_or_else(|_| "8003".into());
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("agent-service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

fn auto_detect_embedding_model(api_url: &str) -> String {
    let url = api_url.to_lowercase();
    if url.contains("openai.com") {
        "text-embedding-3-small".into()
    } else if url.contains("dashscope") {
        "text-embedding-v3".into()
    } else if url.contains("bigmodel.cn") || url.contains("bigmodel.com") {
        "embedding-3".into()
    } else if url.contains("siliconflow") {
        "BAAI/bge-m3".into()
    } else if url.contains("localhost") || url.contains("127.0.0.1") {
        "nomic-embed-text".into()
    } else if url.contains("mistral") {
        "mistral-embed".into()
    } else if url.contains("baichuan") {
        "Baichuan-Text-Embedding".into()
    } else if url.contains("volces.com") {
        "doubao-embedding".into()
    } else {
        "text-embedding-3-small".into()
    }
}

struct NoopEmbeddingGenerator;

#[async_trait::async_trait]
impl domain::ports::EmbeddingGenerator for NoopEmbeddingGenerator {
    async fn generate_embedding(&self, _text: &str) -> anyhow::Result<Vec<f32>> {
        Err(anyhow::anyhow!(
            "Embedding not configured — semantic search disabled"
        ))
    }
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
