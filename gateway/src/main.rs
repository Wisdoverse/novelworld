#![allow(dead_code, unused_imports)]
mod auth;
mod metrics;
mod proxy;
mod setup;

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, get, post},
    Json, Router,
};
use governor::{
    clock::DefaultClock,
    state::{InMemoryState, NotKeyed},
    Quota, RateLimiter,
};
use metrics_exporter_prometheus::PrometheusHandle;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::JwtMiddleware;
use proxy::ServiceProxy;

#[derive(Clone)]
pub struct AppState {
    pub jwt: Arc<JwtMiddleware>,
    pub proxy: Arc<ServiceProxy>,
    pub metrics_handle: PrometheusHandle,
    pub rate_limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
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

    // --- Prometheus metrics ---
    let metrics_handle = metrics::init_metrics();

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

    // --- Global rate limiter: configurable via env, default 500 req/s ---
    let rps: u32 = match std::env::var("RATE_LIMIT_RPS") {
        Ok(v) => match v.parse() {
            Ok(n) => n,
            Err(_) => {
                tracing::warn!("Invalid RATE_LIMIT_RPS value '{}', defaulting to 500", v);
                500
            }
        },
        Err(_) => 500,
    };
    let rate_limiter = Arc::new(RateLimiter::direct(Quota::per_second(
        NonZeroU32::new(rps).expect("RATE_LIMIT_RPS must be > 0"),
    )));

    let state = AppState {
        jwt: jwt.clone(),
        proxy,
        metrics_handle,
        rate_limiter,
    };

    let app = Router::new()
        // --- Observability endpoints (no auth, no rate-limit) ---
        .route("/metrics", get(prometheus_metrics))
        .route("/health", get(health_check))
        // Public routes (no auth)
        .route("/api/auth/register", post(proxy::forward_to_user))
        .route("/api/auth/login", post(proxy::forward_to_user))
        .route("/api/auth/refresh", post(proxy::forward_to_user))
        .route("/api/setup/status", get(setup::get_setup_status))
        .route("/api/setup/test-llm", post(setup::test_llm))
        .route("/api/setup/init", post(setup::init_setup))
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
        // --- Middleware layers (outermost applied first) ---
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .layer(middleware::from_fn(request_id_middleware))
        .layer(middleware::from_fn(metrics::metrics_middleware))
        .layer(
            CorsLayer::new()
                .allow_origin(Any)
                .allow_methods(Any)
                .allow_headers(Any),
        )
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let port = std::env::var("PORT").unwrap_or_else(|_| "8080".into());
    let addr = format!("0.0.0.0:{}", port);
    tracing::info!("Gateway listening on {}", addr);

    let listener = tokio::net::TcpListener::bind(&addr).await?;

    // --- Graceful shutdown ---
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("Gateway shut down cleanly");
    Ok(())
}

// ---------------------------------------------------------------------------
// Prometheus /metrics endpoint
// ---------------------------------------------------------------------------

async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    state.metrics_handle.render()
}

// ---------------------------------------------------------------------------
// Health check with downstream service aggregation
// ---------------------------------------------------------------------------

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let client = &state.proxy.client;
    let (user, novel, agent, narrative) = tokio::join!(
        check_service(client, &state.proxy.user_service_url),
        check_service(client, &state.proxy.novel_service_url),
        check_service(client, &state.proxy.agent_service_url),
        check_service(client, &state.proxy.narrative_service_url),
    );

    let all_healthy = user && novel && agent && narrative;
    let status_code = if all_healthy {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status_code,
        Json(serde_json::json!({
            "status": if all_healthy { "healthy" } else { "degraded" },
            "services": {
                "user": user,
                "novel": novel,
                "agent": agent,
                "narrative": narrative,
            }
        })),
    )
}

async fn check_service(client: &reqwest::Client, base_url: &str) -> bool {
    match client.get(format!("{}/health", base_url))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => true,
        Ok(r) => { tracing::warn!("Health check {} returned {}", base_url, r.status()); false }
        Err(e) => { tracing::warn!("Health check {} failed: {}", base_url, e); false }
    }
}

// ---------------------------------------------------------------------------
// Request-ID middleware: propagate or generate X-Request-Id
// ---------------------------------------------------------------------------

async fn request_id_middleware(mut req: Request, next: Next) -> Response {
    let request_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let hv = match request_id.parse::<axum::http::HeaderValue>() {
        Ok(v) => v,
        Err(_) => {
            let fallback = uuid::Uuid::new_v4().to_string();
            fallback.parse().unwrap() // UUID is always valid
        }
    };

    req.headers_mut().insert("x-request-id", hv.clone());

    let mut response = next.run(req).await;
    response.headers_mut().insert("x-request-id", hv);
    response
}

// ---------------------------------------------------------------------------
// Global rate-limit middleware (token-bucket via governor)
// ---------------------------------------------------------------------------

async fn rate_limit_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    match state.rate_limiter.check() {
        Ok(_) => next.run(req).await,
        Err(_) => (
            StatusCode::TOO_MANY_REQUESTS,
            [("retry-after", "1")],
            "Rate limit exceeded",
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Auth middleware (unchanged logic)
// ---------------------------------------------------------------------------

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
        "/api/setup/status",
        "/api/setup/test-llm",
        "/api/setup/init",
        "/health",
        "/metrics",
    ];
    if public_paths.contains(&path) {
        return next.run(request).await;
    }

    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));

    match auth_header {
        Some(token) => match state.jwt.verify(token) {
            Ok(claims) => {
                let user_id = match claims.sub.parse() {
                    Ok(v) => v,
                    Err(_) => return (StatusCode::UNAUTHORIZED, "Invalid token claims").into_response(),
                };
                let role = match claims.role.parse() {
                    Ok(v) => v,
                    Err(_) => return (StatusCode::UNAUTHORIZED, "Invalid token claims").into_response(),
                };
                request.headers_mut().insert("X-User-Id", user_id);
                request.headers_mut().insert("X-User-Role", role);
                next.run(request).await
            }
            Err(_) => (StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
        },
        None => (StatusCode::UNAUTHORIZED, "Missing Authorization header").into_response(),
    }
}

// ---------------------------------------------------------------------------
// Graceful shutdown signal handler
// ---------------------------------------------------------------------------

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => { tracing::info!("Received SIGINT, starting graceful shutdown"); }
            _ = sigterm.recv() => { tracing::info!("Received SIGTERM, starting graceful shutdown"); }
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.expect("failed to listen for ctrl-c");
        tracing::info!("Received SIGINT, starting graceful shutdown");
    }
}
