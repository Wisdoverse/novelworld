#![allow(dead_code, unused_imports)]
use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use user_service::{
    application::handlers::AuthHandler,
    domain::{self, entities::runtime_config::RuntimeLlmConfig},
    infrastructure::{
        auth::jwt::JwtService,
        llm::LlmClientTester,
        persistence::{pg_user_repo::PgUserRepository, PgReadinessProbe},
        privacy::AgentPrivacyClient,
    },
    interface::http::{router, AppState},
};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    dotenvy::dotenv().ok();
    let metrics = llm_client::install_metrics("user-service")?;

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(&database_url)
        .await?;

    tracing::info!("Connected to PostgreSQL");

    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let access_token_expiry: i64 = std::env::var("AUTH_ACCESS_TOKEN_EXPIRY")
        .unwrap_or_else(|_| "3600".into())
        .parse()
        .unwrap_or(3600);
    let refresh_token_expiry: i64 = std::env::var("AUTH_REFRESH_TOKEN_EXPIRY")
        .unwrap_or_else(|_| "604800".into())
        .parse()
        .unwrap_or(604800);

    let runtime_config_key = std::env::var("RUNTIME_CONFIG_KEY")
        .expect("RUNTIME_CONFIG_KEY must be set to 64 hexadecimal characters");
    let internal_service_token =
        std::env::var("INTERNAL_SERVICE_TOKEN").expect("INTERNAL_SERVICE_TOKEN must be set");
    if internal_service_token.len() < 32 {
        anyhow::bail!("INTERNAL_SERVICE_TOKEN must be at least 32 characters");
    }
    let environment_llm_config = RuntimeLlmConfig::from_environment(
        std::env::var("LLM_API_URL").unwrap_or_else(|_| "https://api.openai.com".into()),
        std::env::var("LLM_MODEL").unwrap_or_else(|_| "gpt-4o-mini".into()),
        std::env::var("LLM_API_KEY").unwrap_or_default(),
    );

    let jwt = Arc::new(JwtService::new(&jwt_secret, access_token_expiry));
    let user_repo = Arc::new(PgUserRepository::new(pool.clone(), &runtime_config_key)?);
    let agent_service_url =
        std::env::var("AGENT_SERVICE_URL").unwrap_or_else(|_| "http://agent-service:8003".into());
    let privacy_cleanup = Arc::new(AgentPrivacyClient::new(
        agent_service_url,
        internal_service_token.clone(),
    )?);

    let token_issuer: Arc<dyn domain::ports::AccessTokenIssuer> = jwt;
    let handler = Arc::new(AuthHandler {
        user_repo,
        jwt: token_issuer,
        llm_tester: Arc::new(LlmClientTester),
        privacy_cleanup,
        environment_llm_config,
        refresh_token_expiry,
    });

    let readiness = Arc::new(PgReadinessProbe::new(pool));
    let state = AppState {
        handler,
        readiness,
        internal_service_token: internal_service_token.into(),
        metrics,
    };

    let app = router(state).layer(
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any),
    );

    let port = std::env::var("PORT").unwrap_or_else(|_| "8001".into());
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("user-service listening on {}", addr);

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
