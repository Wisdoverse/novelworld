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

use agent_service::{
    application::handlers::AgentCommandHandler,
    domain::{self, ports::AccountExportPort, services::memory_manager::MemoryManager},
    infrastructure::{
        cache::{AlwaysReadyProbe, NoopMessageCache, RedisCache, RedisReadinessProbe},
        embedding::{default_model_for_api, EmbeddingAdapter, NoopEmbeddingGenerator},
        http::{narrative_client::NarrativeServiceClient, novel_client::NovelServiceClient},
        llm::LlmAdapter,
        persistence::{
            account_export::PgAccountExport, pg_chat_repo::PgChatRepository,
            pg_memory_repo::PgMemoryRepository, PgReadinessProbe,
        },
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
    let span = tracing::info_span!("service", service = "agent-service", trace_id = %trace_id);
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
    // Initialize logging
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(format!(
            "{},reqwest=off,tower_http=off",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())
        )))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let service_span = tracing::info_span!("service", service = "agent-service", trace_id = "");
    async move {
        // SPEC 14.1: every log entry carries the service name and a trace id
        // (empty outside a request, the propagated X-Trace-Id while handling one).

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

        // Redis is a reconstructable projection. The portable desktop runtime
        // uses the domain-port no-op adapter instead of shipping another daemon.
        let redis_url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://redis:6379".into());
        let (cache, redis_readiness): (
            Arc<dyn domain::ports::MessageCache>,
            Arc<dyn domain::ports::ReadinessProbe>,
        ) = if redis_url == "memory://" {
            tracing::info!("Redis disabled; using PostgreSQL-backed desktop memory path");
            (Arc::new(NoopMessageCache), Arc::new(AlwaysReadyProbe))
        } else {
            let redis_cfg = deadpool_redis::Config::from_url(&redis_url);
            let redis_pool = redis_cfg
                .create_pool(Some(deadpool_redis::Runtime::Tokio1))
                .expect("Failed to create Redis pool");
            tracing::info!("Redis pool created");
            (
                Arc::new(RedisCache::new(redis_pool.clone())),
                Arc::new(RedisReadinessProbe::new(redis_pool)),
            )
        };

        // Shared LLM client (from llm-client workspace crate)
        let api_key = std::env::var("LLM_API_KEY").unwrap_or_default();
        let api_url =
            std::env::var("LLM_API_URL").unwrap_or_else(|_| "https://api.openai.com".into());
        let llm_base = Arc::new(llm_client::RuntimeLlmClient::from_env()?);

        // LLM adapter for chat (TextSummarizer + handler direct calls)
        let llm = Arc::new(LlmAdapter::new(llm_base));

        // Repositories
        let memory_repo = Arc::new(PgMemoryRepository::new(pool.clone()));
        let chat_repo = Arc::new(PgChatRepository::new(pool.clone()));
        let account_export: Arc<dyn AccountExportPort> =
            Arc::new(PgAccountExport::new(pool.clone()));

        // Character info via HTTP to novel-service (replaces direct DB coupling)
        let novel_service_url = std::env::var("NOVEL_SERVICE_URL")
            .unwrap_or_else(|_| "http://novel-service:8002".into());
        let novel_client = Arc::new(NovelServiceClient::new(novel_service_url));
        let character_repo: Arc<dyn domain::repositories::CharacterInfoRepository> =
            novel_client.clone();
        let reading_context: Arc<dyn domain::ports::ReadingContextPort> = novel_client.clone();
        let lore_context: Arc<dyn domain::ports::LoreContextPort> = novel_client.clone();
        let novel_readiness: Arc<dyn domain::ports::ReadinessProbe> = novel_client;
        let narrative_service_url = std::env::var("NARRATIVE_SERVICE_URL")
            .unwrap_or_else(|_| "http://narrative-service:8004".into());
        let narrative_client = Arc::new(NarrativeServiceClient::new(
            narrative_service_url,
            internal_service_token.clone(),
        ));
        let world_context: Arc<dyn domain::ports::WorldContextPort> = narrative_client.clone();
        let narrative_readiness: Arc<dyn domain::ports::ReadinessProbe> = narrative_client;

        // Embedding adapter — auto-select model based on provider
        let embed_api_key = std::env::var("EMBEDDING_API_KEY").unwrap_or_else(|_| api_key.clone());
        let embed_api_url = std::env::var("EMBEDDING_API_URL").unwrap_or_else(|_| api_url.clone());
        let embed_model = std::env::var("EMBEDDING_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| default_model_for_api(&embed_api_url));

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
            cache,
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
            world_context,
            llm: chat_llm,
            // ponytail: process-local admission is sufficient for the current
            // single agent-service replica; use distributed leases when scaling.
            chat_permits: Arc::new(Semaphore::new(8)),
            active_chat_users: Arc::new(Mutex::new(HashSet::new())),
        });

        let state = AppState {
            handler,
            postgres_readiness: Arc::new(PgReadinessProbe::new(pool)),
            redis_readiness,
            novel_readiness,
            narrative_readiness,
            account_export,
            internal_service_token: internal_service_token.into(),
            metrics,
        };

        // Router
        let app = router(state)
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .layer(middleware::from_fn(trace_middleware));

        let port = std::env::var("PORT").unwrap_or_else(|_| "8003".into());
        let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0".into());
        let addr = format!("{}:{}", bind_addr, port);
        tracing::info!("agent-service listening on {}", addr);

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
        _ = sigterm.recv() => { tracing::info!(service = "agent-service", trace_id = "", "Received SIGTERM"); }
    }
    #[cfg(not(unix))]
    ctrl_c.await.ok();
}
