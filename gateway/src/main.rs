#![allow(dead_code, unused_imports)]
mod auth;
mod metrics;
mod proxy;

use anyhow::{bail, Context, Result as AnyResult};
use axum::{
    extract::{ConnectInfo, Request, State},
    http::{header, HeaderName, HeaderValue, Method, StatusCode},
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
use std::future::Future;
use std::net::SocketAddr;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::Instrument;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use auth::JwtMiddleware;
use proxy::ServiceProxy;

#[derive(Clone)]
pub struct AppState {
    pub jwt: Arc<JwtMiddleware>,
    pub proxy: Arc<ServiceProxy>,
    pub metrics_handle: PrometheusHandle,
    pub rate_limiter: Arc<RateLimiter<NotKeyed, InMemoryState, DefaultClock>>,
    pub account_export_permits: Arc<Semaphore>,
    readiness_cache: Arc<ReadinessCache>,
}

const READINESS_CACHE_TTL: Duration = Duration::from_secs(1);

#[derive(Clone, Copy)]
struct ReadinessSnapshot {
    checked_at: Instant,
    user: bool,
    novel: bool,
    agent: bool,
    narrative: bool,
}

struct ReadinessCache {
    snapshot: Mutex<Option<ReadinessSnapshot>>,
}

impl ReadinessCache {
    fn new() -> Self {
        Self {
            snapshot: Mutex::new(None),
        }
    }

    async fn get_or_refresh<F, Fut>(&self, refresh: F) -> ReadinessSnapshot
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = ReadinessSnapshot>,
    {
        let mut cached = self.snapshot.lock().await;
        if let Some(snapshot) = *cached {
            if snapshot.checked_at.elapsed() < READINESS_CACHE_TTL {
                return snapshot;
            }
        }

        let snapshot = refresh().await;
        *cached = Some(snapshot);
        snapshot
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    run_body().await
}

async fn run_body() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(format!(
            "{},reqwest=off,tower_http=off",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into())
        )))
        .with(tracing_subscriber::fmt::layer().json())
        .init();

    let service_span = tracing::info_span!("service", service = "gateway", trace_id = "");
    async move {
        dotenvy::dotenv().ok();

        // --- Prometheus metrics ---
        let metrics_handle = metrics::init_metrics();

        let jwt_secret = std::env::var("JWT_SECRET").expect("JWT_SECRET must be set");
        let jwt = Arc::new(JwtMiddleware::new(&jwt_secret));
        let internal_service_token =
            std::env::var("INTERNAL_SERVICE_TOKEN").expect("INTERNAL_SERVICE_TOKEN must be set");
        if internal_service_token.len() < 32 {
            anyhow::bail!("INTERNAL_SERVICE_TOKEN must be at least 32 characters");
        }

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
            internal_service_token: internal_service_token.into(),
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
            // ponytail: The production topology has one Gateway process. Add
            // distributed admission only when Gateway replicas actually exist.
            account_export_permits: Arc::new(Semaphore::new(2)),
            readiness_cache: Arc::new(ReadinessCache::new()),
        };

        let app = build_router(state)?;

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
    .instrument(service_span)
    .await
}

// ---------------------------------------------------------------------------
// Prometheus /metrics endpoint
// ---------------------------------------------------------------------------

async fn prometheus_metrics(State(state): State<AppState>) -> impl IntoResponse {
    state.metrics_handle.render()
}

// ---------------------------------------------------------------------------
// Liveness and readiness
// ---------------------------------------------------------------------------

async fn liveness_check() -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({"status": "alive"})))
}

async fn readiness_check(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot = state
        .readiness_cache
        .get_or_refresh(|| async {
            let client = &state.proxy.client;
            let (user, novel, agent, narrative) = tokio::join!(
                check_service(client, &state.proxy.user_service_url),
                check_service(client, &state.proxy.novel_service_url),
                check_service(client, &state.proxy.agent_service_url),
                check_service(client, &state.proxy.narrative_service_url),
            );
            ReadinessSnapshot {
                checked_at: Instant::now(),
                user,
                novel,
                agent,
                narrative,
            }
        })
        .await;

    let all_healthy = snapshot.user && snapshot.novel && snapshot.agent && snapshot.narrative;
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
                "user": snapshot.user,
                "novel": snapshot.novel,
                "agent": snapshot.agent,
                "narrative": snapshot.narrative,
            }
        })),
    )
}

async fn check_service(client: &reqwest::Client, base_url: &str) -> bool {
    match client
        .get(format!("{}/ready", base_url))
        .timeout(Duration::from_secs(3))
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => true,
        Ok(r) => {
            tracing::warn!("Health check {} returned {}", base_url, r.status());
            false
        }
        Err(e) => {
            tracing::warn!("Health check {} failed: {}", base_url, e);
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Request-ID middleware: propagate or generate X-Request-Id
// ---------------------------------------------------------------------------

/// SPEC 14.1: every request carries an X-Trace-Id (accepted from the
/// caller or generated), the response echoes it, the header is forwarded to
/// downstream services by the proxy, and every log emitted while handling the
/// request is wrapped in a span carrying the trace id.
async fn request_id_middleware(mut req: Request, next: Next) -> Response {
    let trace_id = req
        .headers()
        .get("x-trace-id")
        .and_then(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let hv = match trace_id.parse::<axum::http::HeaderValue>() {
        Ok(v) => v,
        Err(_) => {
            let fallback = uuid::Uuid::new_v4().to_string();
            fallback.parse().unwrap() // UUID is always valid
        }
    };

    req.headers_mut().insert("x-trace-id", hv.clone());
    let span = tracing::info_span!(
        "service",
        service = "gateway",
        trace_id = %hv.to_str().unwrap_or_default()
    );

    let mut response = async {
        let response = next.run(req).await;
        // SPEC 14.1: one request-scoped entry per request, so the log contract
        // holds even when no other gateway event fires while handling it.
        tracing::info!("request completed");
        response
    }
    .instrument(span)
    .await;
    response.headers_mut().insert("x-trace-id", hv);
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
    if is_observability_path(req.uri().path()) {
        return next.run(req).await;
    }

    match state.rate_limiter.check() {
        Ok(_) => next.run(req).await,
        Err(_) => {
            let mut response = proxy::api_error_response(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Rate limit exceeded",
            );
            response
                .headers_mut()
                .insert("retry-after", axum::http::HeaderValue::from_static("1"));
            response
        }
    }
}

fn is_observability_path(path: &str) -> bool {
    matches!(path, "/live" | "/health" | "/ready" | "/metrics")
}

// ---------------------------------------------------------------------------
// Auth middleware (unchanged logic)
// ---------------------------------------------------------------------------

fn build_router(state: AppState) -> AnyResult<Router> {
    Ok(Router::new()
        // --- Observability endpoints (no auth, no rate-limit) ---
        .route("/metrics", get(prometheus_metrics))
        .route("/live", get(liveness_check))
        .route("/health", get(readiness_check))
        .route("/ready", get(readiness_check))
        // Public routes (no auth)
        .route("/api/auth/register", post(proxy::forward_to_user))
        .route("/api/auth/login", post(proxy::forward_to_user))
        .route("/api/auth/refresh", post(proxy::forward_to_user))
        .route("/api/setup/status", get(proxy::forward_to_user))
        .route("/api/setup/init", post(proxy::forward_to_user))
        // Protected routes
        .route(
            "/api/auth/me",
            get(proxy::forward_to_user).delete(proxy::forward_to_user),
        )
        .route("/api/auth/logout", post(proxy::forward_to_user))
        .route("/api/account/export", get(proxy::export_account))
        .route("/api/settings/{*path}", any(proxy::forward_to_user))
        .route("/api/novels", post(proxy::forward_to_novel))
        .route("/api/novels", get(proxy::forward_to_novel))
        .route("/api/novels/{*path}", any(proxy::forward_to_novel))
        .route("/api/chat/{*path}", any(proxy::forward_to_agent))
        .route("/api/memories/{*path}", any(proxy::forward_to_agent))
        .route("/api/narrative/{*path}", any(proxy::forward_to_narrative))
        .route("/api/progress/{*path}", any(proxy::forward_to_novel))
        .route("/api/users/{*path}", any(proxy::forward_to_user))
        .route("/api/characters/{*path}", any(proxy::forward_to_novel))
        // Apply the global backstop inside authentication so rejected protected
        // requests cannot spend shared capacity.
        .layer(middleware::from_fn_with_state(
            state.clone(),
            rate_limit_middleware,
        ))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(middleware::from_fn(request_id_middleware))
        .layer(middleware::from_fn(metrics::metrics_middleware))
        .layer(cors_layer(cors_origins()?))
        .layer(TraceLayer::new_for_http())
        .with_state(state))
}

/// Paths that skip authentication. This is the security-critical half of
/// the route/resource authorization matrix: adding a path here exposes it,
/// so the authz matrix tests pin this set exactly.
const PUBLIC_PATHS: &[&str] = &[
    "/api/auth/register",
    "/api/auth/login",
    "/api/auth/refresh",
    "/api/setup/status",
    "/api/setup/init",
    "/live",
    "/health",
    "/ready",
    "/metrics",
];

/// Route families that must require authentication. The gateway backstop
/// already 401s anything not in PUBLIC_PATHS; this table pins the intent so
/// the matrix tests fail loudly if a family becomes public.
const PROTECTED_ROUTE_FAMILIES: &[&str] = &[
    "/api/auth/me",
    "/api/auth/logout",
    "/api/account/export",
    "/api/settings/",
    "/api/novels",
    "/api/chat/",
    "/api/memories/",
    "/api/narrative/",
    "/api/progress/",
    "/api/users/",
    "/api/characters/",
];

async fn auth_middleware(
    State(state): State<AppState>,
    mut request: Request,
    next: Next,
) -> Response {
    let path = request.uri().path();

    // Public routes skip auth
    if PUBLIC_PATHS.contains(&path) {
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
                    Err(_) => {
                        return proxy::api_error_response(
                            StatusCode::UNAUTHORIZED,
                            "unauthorized",
                            "Invalid token claims",
                        )
                    }
                };
                let role = match claims.role.parse() {
                    Ok(v) => v,
                    Err(_) => {
                        return proxy::api_error_response(
                            StatusCode::UNAUTHORIZED,
                            "unauthorized",
                            "Invalid token claims",
                        )
                    }
                };
                request.headers_mut().insert("X-User-Id", user_id);
                request.headers_mut().insert("X-User-Role", role);
                next.run(request).await
            }
            Err(_) => {
                proxy::api_error_response(StatusCode::UNAUTHORIZED, "unauthorized", "Invalid token")
            }
        },
        None => proxy::api_error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing Authorization header",
        ),
    }
}

// ---------------------------------------------------------------------------
// Graceful shutdown signal handler
// ---------------------------------------------------------------------------

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => { tracing::info!("Received SIGINT, starting graceful shutdown"); }
            _ = sigterm.recv() => { tracing::info!(service = "gateway", trace_id = "", "Received SIGTERM, starting graceful shutdown"); }
        }
    }

    #[cfg(not(unix))]
    {
        ctrl_c.await.expect("failed to listen for ctrl-c");
        tracing::info!("Received SIGINT, starting graceful shutdown");
    }
}

/// The preview CORS allowlist: browsers may reach the gateway cross-origin
/// only from the documented preview origins. Production requests are
/// same-origin through Nginx, so this only gates dev-mode and operator-tunnel
/// origins. A malformed or empty list fails startup: CORS is security-relevant
/// posture, never silently widened.
fn cors_origins() -> AnyResult<Vec<HeaderValue>> {
    let raw = std::env::var("CORS_ORIGINS").unwrap_or_else(|_| {
        "http://localhost:5173,http://127.0.0.1:5173,http://localhost,http://127.0.0.1".into()
    });
    parse_cors_origins(&raw)
}

fn parse_cors_origins(raw: &str) -> AnyResult<Vec<HeaderValue>> {
    let origins = raw
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .map(|origin| {
            let uri = origin
                .parse::<axum::http::Uri>()
                .with_context(|| format!("CORS_ORIGINS contains an invalid origin: {origin}"))?;
            let http_scheme = uri
                .scheme_str()
                .is_some_and(|scheme| matches!(scheme, "http" | "https"));
            // A trailing slash or fragment never matches a browser Origin
            // (origins carry neither); reject it so operator typos fail loudly
            // instead of silently narrowing the allowlist.
            if !http_scheme
                || uri.authority().is_none()
                || uri.path() != "/"
                || uri.query().is_some()
                || origin.contains('#')
                || origin.ends_with('/')
            {
                bail!("CORS_ORIGINS origins must be http(s) origins without a path, query, or fragment: {origin}");
            }
            HeaderValue::from_str(origin)
                .with_context(|| format!("CORS_ORIGINS contains an invalid origin: {origin}"))
        })
        .collect::<AnyResult<Vec<_>>>()?;
    if origins.is_empty() {
        bail!("CORS_ORIGINS must name at least one origin");
    }
    Ok(origins)
}

fn cors_layer(origins: Vec<HeaderValue>) -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            HeaderName::from_static("idempotency-key"),
        ])
}

#[cfg(test)]
mod authz_matrix_tests {
    use super::*;
    use auth::Claims;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use jsonwebtoken::{encode, EncodingKey, Header as JwtHeader};
    use tower::ServiceExt;

    const TEST_SECRET: &str = "test-secret-test-secret-test-secret-test-secret";

    fn test_state() -> AppState {
        // build_recorder does not install a global recorder, so parallel tests
        // never race the one-time metrics installation; leaked on purpose so
        // the handle stays valid for the test's lifetime.
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let metrics_handle = recorder.handle();
        Box::leak(Box::new(recorder));
        AppState {
            jwt: Arc::new(JwtMiddleware::new(TEST_SECRET)),
            proxy: Arc::new(ServiceProxy {
                novel_service_url: "http://127.0.0.1:1".into(),
                agent_service_url: "http://127.0.0.1:1".into(),
                narrative_service_url: "http://127.0.0.1:1".into(),
                user_service_url: "http://127.0.0.1:1".into(),
                client: reqwest::Client::new(),
                internal_service_token: "x".repeat(32).into(),
            }),
            metrics_handle,
            rate_limiter: Arc::new(RateLimiter::direct(Quota::per_second(
                NonZeroU32::new(10_000).expect("static quota is non-zero"),
            ))),
            account_export_permits: Arc::new(Semaphore::new(2)),
            readiness_cache: Arc::new(ReadinessCache::new()),
        }
    }

    async fn request(
        router: &Router,
        method: Method,
        path: &str,
        bearer: Option<&str>,
    ) -> StatusCode {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(token) = bearer {
            builder = builder.header("Authorization", format!("Bearer {token}"));
        }
        let response = router
            .clone()
            .oneshot(builder.body(Body::empty()).expect("request builds"))
            .await
            .expect("router responds");
        response.status()
    }

    fn protected_representatives() -> [(Method, &'static str); 12] {
        [
            (Method::GET, "/api/auth/me"),
            (Method::POST, "/api/auth/logout"),
            (Method::GET, "/api/account/export"),
            (Method::GET, "/api/settings/profile"),
            (Method::GET, "/api/novels"),
            (Method::GET, "/api/novels/some-id"),
            (Method::GET, "/api/chat/some-id/history"),
            (Method::GET, "/api/memories/some-id"),
            (Method::GET, "/api/narrative/some-id/world"),
            (Method::GET, "/api/progress/some-id"),
            (Method::GET, "/api/users/me"),
            (Method::GET, "/api/characters/some-id"),
        ]
    }

    fn valid_token() -> String {
        let now = chrono::Utc::now().timestamp();
        encode(
            &JwtHeader::default(),
            &Claims {
                sub: "3d2329c2-05d4-48b1-9ab4-42927c3efff5".into(),
                email: "admin@test.invalid".into(),
                role: "admin".into(),
                iat: now,
                exp: now + 3_600,
            },
            &EncodingKey::from_secret(TEST_SECRET.as_bytes()),
        )
        .expect("token signs")
    }

    #[test]
    fn public_paths_are_pinned_and_families_never_overlap() {
        assert_eq!(
            PUBLIC_PATHS,
            &[
                "/api/auth/register",
                "/api/auth/login",
                "/api/auth/refresh",
                "/api/setup/status",
                "/api/setup/init",
                "/live",
                "/health",
                "/ready",
                "/metrics",
            ]
        );
        for family in PROTECTED_ROUTE_FAMILIES {
            assert!(
                !PUBLIC_PATHS.contains(family),
                "{family} must not be public"
            );
            assert!(
                !PUBLIC_PATHS
                    .iter()
                    .any(|path| path.starts_with(family.trim_end_matches('/'))),
                "no public path may fall under protected family {family}"
            );
            let covered = protected_representatives().iter().any(|(_, path)| {
                if family.ends_with('/') {
                    path.starts_with(family)
                } else {
                    *path == *family || path.starts_with(&format!("{family}/"))
                }
            });
            assert!(
                covered,
                "protected family {family} lacks a behavioral representative"
            );
        }
    }

    #[tokio::test]
    async fn protected_families_reject_missing_and_garbage_tokens() {
        let router = build_router(test_state()).unwrap();
        for (method, path) in protected_representatives() {
            let missing = request(&router, method.clone(), path, None).await;
            assert_eq!(
                missing,
                StatusCode::UNAUTHORIZED,
                "{path} without a token must be 401"
            );
            let garbage = request(&router, method, path, Some("garbage.token.value")).await;
            assert_eq!(
                garbage,
                StatusCode::UNAUTHORIZED,
                "{path} with a garbage token must be 401"
            );
        }
    }

    #[tokio::test]
    async fn public_paths_skip_auth() {
        let router = build_router(test_state()).unwrap();
        let cases = [
            (Method::POST, "/api/auth/register"),
            (Method::POST, "/api/auth/login"),
            (Method::POST, "/api/auth/refresh"),
            (Method::GET, "/api/setup/status"),
            (Method::POST, "/api/setup/init"),
            (Method::GET, "/live"),
            (Method::GET, "/health"),
            (Method::GET, "/ready"),
            (Method::GET, "/metrics"),
        ];
        for (method, path) in cases {
            let status = request(&router, method, path, None).await;
            assert_ne!(
                status,
                StatusCode::UNAUTHORIZED,
                "{path} must skip authentication"
            );
        }
    }

    #[tokio::test]
    async fn valid_tokens_pass_the_gate() {
        let router = build_router(test_state()).unwrap();
        let status = request(&router, Method::GET, "/api/novels", Some(&valid_token())).await;
        assert_ne!(
            status,
            StatusCode::UNAUTHORIZED,
            "a valid token must pass authentication"
        );
    }
}

#[cfg(test)]
mod cors_tests {
    use super::{cors_layer, parse_cors_origins};
    use axum::{
        body::Body,
        http::{Method, Request, Response, StatusCode},
    };
    use std::convert::Infallible;
    use tower::service_fn;
    use tower::{ServiceBuilder, ServiceExt};

    #[test]
    fn parses_the_preview_origin_list() {
        let origins =
            parse_cors_origins("http://localhost:5173, http://127.0.0.1:5173,,http://localhost")
                .unwrap();
        assert_eq!(origins.len(), 3);
        assert!(parse_cors_origins("not a valid origin value").is_err());
        assert!(parse_cors_origins(" , ").is_err());
        assert!(parse_cors_origins("http://localhost:5173/").is_err());
        assert!(parse_cors_origins("http://localhost:5173#fragment").is_err());
        assert!(parse_cors_origins("http://localhost:5173/api").is_err());
    }

    async fn preflight_response(origin: &str) -> Response<Body> {
        let origins = parse_cors_origins(
            "http://localhost:5173,http://127.0.0.1:5173,http://localhost,http://127.0.0.1",
        )
        .unwrap();
        let service = ServiceBuilder::new()
            .layer(cors_layer(origins))
            .service(service_fn(|_: Request<Body>| async {
                Ok::<_, Infallible>(axum::response::Response::new(Body::empty()))
            }));
        let request = Request::builder()
            .method(Method::OPTIONS)
            .header("Origin", origin)
            .header("Access-Control-Request-Method", "POST")
            .uri("/api/auth/login")
            .body(Body::empty())
            .unwrap();
        service.oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn preview_origins_pass_preflight_and_foreign_origins_do_not() {
        let allowed = preflight_response("http://localhost:5173").await;
        assert_eq!(allowed.status(), StatusCode::OK);
        assert_eq!(
            allowed
                .headers()
                .get("access-control-allow-origin")
                .unwrap(),
            "http://localhost:5173"
        );

        let denied = preflight_response("http://evil.example").await;
        assert!(
            denied
                .headers()
                .get("access-control-allow-origin")
                .is_none(),
            "a foreign origin must not receive a CORS allowance"
        );
    }
}

#[cfg(test)]
mod observability_tests {
    use super::{is_observability_path, ReadinessCache, ReadinessSnapshot};
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    use std::time::Instant;

    #[test]
    fn probes_and_metrics_do_not_spend_rate_limit_capacity() {
        for path in ["/live", "/health", "/ready", "/metrics"] {
            assert!(is_observability_path(path));
        }
        assert!(!is_observability_path("/api/novels"));
    }

    #[tokio::test]
    async fn concurrent_probe_requests_share_one_dependency_refresh() {
        let cache = Arc::new(ReadinessCache::new());
        let refreshes = Arc::new(AtomicUsize::new(0));
        let mut tasks = Vec::new();

        for _ in 0..20 {
            let cache = cache.clone();
            let refreshes = refreshes.clone();
            tasks.push(tokio::spawn(async move {
                cache
                    .get_or_refresh(|| async {
                        refreshes.fetch_add(1, Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        ReadinessSnapshot {
                            checked_at: Instant::now(),
                            user: true,
                            novel: true,
                            agent: true,
                            narrative: true,
                        }
                    })
                    .await
            }));
        }

        for task in tasks {
            assert!(task.await.unwrap().user);
        }
        assert_eq!(refreshes.load(Ordering::SeqCst), 1);
    }
}
