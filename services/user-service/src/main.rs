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

use application::handlers::AuthHandler;
use infrastructure::auth::jwt::JwtService;
use infrastructure::persistence::pg_user_repo::PgUserRepository;
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

    let jwt = Arc::new(JwtService::new(&jwt_secret, access_token_expiry));
    let user_repo = Arc::new(PgUserRepository::new(pool));

    let handler = Arc::new(AuthHandler {
        user_repo,
        jwt,
        refresh_token_expiry,
    });

    let state = AppState { handler };

    let app = router(state)
        .layer(CorsLayer::new().allow_origin(Any).allow_methods(Any).allow_headers(Any));

    let port = std::env::var("PORT").unwrap_or_else(|_| "8001".into());
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("user-service listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
