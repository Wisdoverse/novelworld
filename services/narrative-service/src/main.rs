#![allow(dead_code, unused_imports)]
use anyhow::Result;
use axum::{extract::Request, middleware, middleware::Next, response::Response};
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing::Instrument;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use narrative_service::{
    application::handlers::NarrativeCommandHandler,
    domain,
    infrastructure::{
        dice::Sha256DiceRoller,
        http::{
            agent_client::AgentServiceClient, laya_client::LayaActionSuggester,
            novel_client::NovelServiceClient,
        },
        llm::LlmAdapter,
        persistence::{
            account_export::PgAccountExport,
            pg_narrative_repo::{
                PgNarrativeNodeRepository, PgPlayerChapterRepository, PgUserChoiceRepository,
            },
            pg_world_state_repo::PgWorldStateRepository,
            pg_world_turn_repo::PgWorldTurnRepository,
            PgReadinessProbe,
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
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(axum::extract::MatchedPath::as_str)
        .unwrap_or("unmatched")
        .to_owned();
    let start = std::time::Instant::now();
    let span = tracing::info_span!(target: "novelworld_context", "service", service = "narrative-service", trace_id = %trace_id);
    async {
        let response = next.run(request).await;
        let status = response.status().as_u16();
        let elapsed_ms = start.elapsed().as_millis() as u64;
        if status >= 500 {
            tracing::error!(%method, %route, status, elapsed_ms, "request completed");
        } else if status == 429 {
            tracing::warn!(%method, %route, status, elapsed_ms, "request completed");
        } else {
            tracing::info!(%method, %route, status, elapsed_ms, "request completed");
        }
        response
    }
    .instrument(span)
    .await
}

#[tokio::main]
async fn main() -> Result<()> {
    if let Some(output) =
        llm_client::diagnostic_capability::capability_probe(std::env::args_os().skip(1))
    {
        println!("{output}");
        return Ok(());
    }
    run_body().await
}

async fn run_body() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(format!(
            "{},reqwest=off,tower_http=off,novelworld_context=info",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())
        )))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let service_span = tracing::info_span!(target: "novelworld_context", "service", service = "narrative-service", trace_id = "");
    async move {
        // SPEC 14.1: every log entry carries the service name and a trace id
        // (empty outside a request, the propagated X-Trace-Id while handling one).

        dotenvy::dotenv().ok();
        let metrics = llm_client::install_metrics("narrative-service")?;
        let internal_service_token =
            std::env::var("INTERNAL_SERVICE_TOKEN").expect("INTERNAL_SERVICE_TOKEN must be set");
        llm_client::validate_internal_service_token(&internal_service_token)?;

        // Database connection pool
        let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
        let pool = PgPoolOptions::new()
            .max_connections(20)
            .connect(&database_url)
            .await?;

        tracing::info!("Connected to PostgreSQL");

        // LLM client (shared workspace crate, behind domain port trait)
        let llm: Arc<dyn domain::ports::LlmPort> = Arc::new(LlmAdapter::new(Arc::new(
            llm_client::RuntimeLlmClient::from_env()?,
        )));

        // Repositories
        let node_repo = Arc::new(PgNarrativeNodeRepository::new(pool.clone()));
        let choice_repo = Arc::new(PgUserChoiceRepository::new(pool.clone()));
        let world_state_repo = Arc::new(PgWorldStateRepository::new(pool.clone()));
        let world_turn_repo = Arc::new(PgWorldTurnRepository::new(pool.clone()));
        let player_chapter_repo = Arc::new(PgPlayerChapterRepository::new(pool.clone()));
        let account_export: Arc<dyn domain::ports::AccountExportPort> =
            Arc::new(PgAccountExport::new(pool.clone()));
        let novel_service_url = std::env::var("NOVEL_SERVICE_URL")
            .unwrap_or_else(|_| "http://novel-service:8002".into());
        let chapter_repo = Arc::new(NovelServiceClient::new(
            novel_service_url,
            internal_service_token.clone(),
        ));
        let novel_readiness: Arc<dyn domain::ports::ReadinessProbe> = chapter_repo.clone();
        let agent_service_url = std::env::var("AGENT_SERVICE_URL")
            .unwrap_or_else(|_| "http://agent-service:8003".into());
        let agent_memory: Arc<dyn domain::ports::AgentMemoryPort> = Arc::new(
            AgentServiceClient::new(agent_service_url, internal_service_token.clone()),
        );
        let dice_roller: Arc<dyn domain::ports::DiceRollerPort> = Arc::new(Sha256DiceRoller::new(
            internal_service_token.as_bytes().to_vec(),
        )?);

        // Application handler
        let handler = Arc::new(NarrativeCommandHandler {
            node_repo,
            choice_repo,
            character_context_repo: world_state_repo.clone(),
            world_state_repo,
            world_turn_repo,
            player_chapter_repo,
            chapter_repo,
            llm,
            agent_memory,
            dice_roller,
        });
        let _memory_projection_recovery = handler.spawn_memory_projection_recovery();

        let action_suggester: Option<Arc<dyn domain::ports::ActionSuggestionPort>> = match (
            std::env::var("LAYA_API_URL")
                .ok()
                .filter(|value| !value.trim().is_empty()),
            std::env::var("LAYA_API_KEY")
                .ok()
                .filter(|value| !value.is_empty()),
        ) {
            (Some(url), Some(key)) => match LayaActionSuggester::new(&url, key) {
                Ok(client) => Some(Arc::new(client)),
                Err(_) => {
                    tracing::warn!("Laya action suggestions disabled: invalid configuration");
                    None
                }
            },
            (None, None) => None,
            _ => {
                tracing::warn!("Laya action suggestions disabled: URL and key are both required");
                None
            }
        };

        let state = AppState {
            handler,
            postgres_readiness: Arc::new(PgReadinessProbe::new(pool)),
            novel_readiness,
            account_export,
            action_suggester,
            internal_service_token: internal_service_token.into(),
            metrics,
        };

        // Router with CORS
        let app = router(state)
            .layer(
                CorsLayer::new()
                    .allow_origin(Any)
                    .allow_methods(Any)
                    .allow_headers(Any),
            )
            .layer(middleware::from_fn(trace_middleware));

        let port = std::env::var("PORT").unwrap_or_else(|_| "8004".into());
        let bind_addr = std::env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0".into());
        let addr = format!("{}:{}", bind_addr, port);
        tracing::info!("narrative-service listening on {}", addr);

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
        _ = sigterm.recv() => { tracing::info!(service = "narrative-service", trace_id = "", "Received SIGTERM"); }
    }
    #[cfg(not(unix))]
    ctrl_c.await.ok();
}
