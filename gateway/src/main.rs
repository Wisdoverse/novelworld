mod auth;
mod proxy;

use axum::{
    extract::{Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post, put, delete},
    Router,
};
use std::sync::Arc;
use tower_http::cors::{CorsLayer, Any};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::JwtMiddleware;
use proxy::ServiceProxy;

#[derive(Clone)]
pub struct AppState {
    pub jwt: Arc<JwtMiddleware>,
    pub proxy: Arc<ServiceProxy>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    dotenvy::dotenv().ok();

    let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
    let jwt = Arc::new(JwtMiddleware::new(&jwt_secret));

    let proxy = Arc::new(ServiceProxy {
        novel_service_url: std::env::var("NOVEL_SERVICE_URL")
            .unwrap_or_else(|_| "http://novel-service:8002".into()),
        agent_service_url: std::env::var("AGENT_SERVICE_URL")
            .unwrap_or_else(|_| "http://agent-service:8003".into()),
        narrative_service_url: std::env::var("NARRATIVE_SERVICE_URL")
            .unwrap_or_else(|_| "http://narrative-service:8004".into()),
        user_service_url: std::env::var("USER_SERVICE_URL")
            .unwrap_or_else(|_| "http://user-service:8001".into()),
        client: reqwest::Client::new(),
    });

    let state = AppState { jwt: jwt.clone(), proxy };

    let app = Router::new()
        // Public routes (no auth)
        .route("/api/auth/register", post(proxy::forward_to_user))
        .route("/api/auth/login", post(proxy::forward_to_user))
        .route("/api/auth/refresh", post(proxy::forward_to_user))
        .route("/health", get(health_check))
        // Protected routes
        .route("/api/auth/me", get(proxy::forward_to_user))
        .route("/api/auth/logout", post(proxy::forward_to_user))
        .route("/api/novels", post(proxy::forward_to_novel))
        .route("/api/novels", get(proxy::forward_to_novel))
        .route("/api/novels/{*path}", any(proxy::forward_to_novel))
        .route("/api/chat/{*path}", any(proxy::forward_to_agent))
        .route("/api/memories/{*path}", any(proxy::forward_to_agent))
        .route("/api/narrative/{*path}", any(proxy::forward_to_narrative))
        .route("/api/progress/{*path}", any(proxy::forward_to_novel))
        .route("/api/users/{*path}", any(proxy::forward_to_user))
        .route("/api/characters/{*path}", any(proxy::forward_to_novel))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("Gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Public routes skip auth
    let public_paths = [
        "/api/auth/register",
        "/api/auth/login",
        "/api/auth/refresh",
        "/health",
    ];
    if public_paths.iter().any(|p| path == *p) {
        return next.run(request).await;
    }

    let auth_header = request.headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match auth_header {
        Some(token) => {
            match state.jwt.verify(token) {
                Ok(claims) => {
                    request.headers_mut().insert(
                        "X-User-Id",
                        claims.sub.parse().unwrap(),
                    );
                    request.headers_mut().insert(
                        "X-User-Role",
                        claims.role.parse().unwrap(),
                    );
                    next.run(request).await
                }
                Err(_) => (StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
            }
        }
        None => (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response(),
    }
}
