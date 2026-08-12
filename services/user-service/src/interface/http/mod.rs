use axum::{
    extract::{DefaultBodyLimit, Json, State},
    http::{header::CACHE_CONTROL, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::application::handlers::{AuthError, AuthHandler};
use crate::domain::ports::ReadinessProbe;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<AuthHandler>,
    pub readiness: Arc<dyn ReadinessProbe>,
    pub internal_service_token: Arc<str>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/me", get(get_me))
        .route("/auth/logout", post(logout))
        .route("/setup/status", get(setup_status))
        .route("/setup/init", post(setup_init))
        .route("/internal/runtime/llm", get(runtime_llm_config))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterRequest {
    email: String,
    password: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LogoutRequest {
    refresh_token: String,
}

#[derive(Debug, Serialize)]
struct AuthResponse {
    user: UserDto,
    access_token: String,
    refresh_token: String,
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Serialize)]
struct UserDto {
    id: Uuid,
    email: String,
    name: Option<String>,
    avatar_url: Option<String>,
    role: String,
}

#[derive(Debug, Serialize)]
struct ApiError {
    error: ErrorDetail,
}

#[derive(Debug, Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

#[derive(Debug, Serialize)]
struct SetupStatus {
    configured: bool,
    admin_configured: bool,
    llm_configured: bool,
    contract: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupRequest {
    email: String,
    password: String,
    name: Option<String>,
    provider: Option<String>,
    api_key: Option<String>,
}

#[derive(Serialize)]
struct RuntimeLlmConfigResponse {
    contract: u8,
    api_url: String,
    model: String,
    api_key: String,
}

fn user_dto(u: &crate::domain::entities::user::User) -> UserDto {
    UserDto {
        id: u.id,
        email: u.email.clone(),
        name: u.name.clone(),
        avatar_url: u.avatar_url.clone(),
        role: u.role.as_str().to_string(),
    }
}

fn error_response(status: StatusCode, code: &'static str, msg: &str) -> impl IntoResponse {
    (
        status,
        Json(ApiError {
            error: ErrorDetail {
                code,
                message: msg.to_string(),
            },
        }),
    )
}

fn auth_error_response(error: AuthError) -> axum::response::Response {
    let (status, code, message) = match error {
        AuthError::Validation(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            message,
        ),
        AuthError::EmailAlreadyRegistered => (
            StatusCode::CONFLICT,
            "conflict",
            "Email already registered".into(),
        ),
        AuthError::AlreadyConfigured => (
            StatusCode::CONFLICT,
            "conflict",
            "Setup is already complete".into(),
        ),
        AuthError::SetupRequired => (
            StatusCode::CONFLICT,
            "setup_required",
            "Create the initial administrator before registering users".into(),
        ),
        AuthError::LlmUnavailable => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "llm_unavailable",
            "The AI provider rejected the key or is unavailable".into(),
        ),
        AuthError::InvalidCredentials => (
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid email or password".into(),
        ),
        AuthError::InvalidRefreshToken => (
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid or expired refresh token".into(),
        ),
        AuthError::NotFound => (StatusCode::NOT_FOUND, "not_found", "User not found".into()),
        AuthError::Internal(error) => {
            tracing::error!(error = ?error, "user-service operation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Authentication operation failed".into(),
            )
        }
    };
    error_response(status, code, &message).into_response()
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    match state
        .handler
        .register(&req.email, &req.password, req.name)
        .await
    {
        Ok((user, access_token, refresh_token)) => (
            StatusCode::CREATED,
            Json(AuthResponse {
                user: user_dto(&user),
                access_token,
                refresh_token,
            }),
        )
            .into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn login(State(state): State<AppState>, Json(req): Json<LoginRequest>) -> impl IntoResponse {
    match state.handler.login(&req.email, &req.password).await {
        Ok((user, access_token, refresh_token)) => (
            StatusCode::OK,
            Json(AuthResponse {
                user: user_dto(&user),
                access_token,
                refresh_token,
            }),
        )
            .into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> impl IntoResponse {
    match state.handler.refresh(&req.refresh_token).await {
        Ok(access_token) => (StatusCode::OK, Json(TokenResponse { access_token })).into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn get_me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or invalid user identity",
            )
            .into_response()
        }
    };
    match state.handler.get_me(user_id).await {
        Ok(user) => (StatusCode::OK, Json(user_dto(&user))).into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn logout(
    State(state): State<AppState>,
    Json(req): Json<LogoutRequest>,
) -> impl IntoResponse {
    match state.handler.logout(&req.refresh_token).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn setup_status(State(state): State<AppState>) -> impl IntoResponse {
    match state.handler.setup_status().await {
        Ok(status) => (
            StatusCode::OK,
            Json(SetupStatus {
                configured: status.configured(),
                admin_configured: status.admin_configured,
                llm_configured: status.llm_configured,
                contract: 3,
            }),
        )
            .into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn setup_init(
    State(state): State<AppState>,
    Json(req): Json<SetupRequest>,
) -> impl IntoResponse {
    match state
        .handler
        .setup(
            &req.email,
            &req.password,
            req.name,
            req.provider.as_deref(),
            req.api_key.as_deref(),
        )
        .await
    {
        Ok((user, access_token, refresh_token)) => (
            StatusCode::CREATED,
            Json(AuthResponse {
                user: user_dto(&user),
                access_token,
                refresh_token,
            }),
        )
            .into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn runtime_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let authorized = headers
        .get("X-Internal-Service-Token")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| secrets_equal(value, state.internal_service_token.as_ref()));
    if !authorized {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid internal service identity",
        )
        .into_response();
    }

    match state.handler.runtime_llm_config().await {
        Ok(config) => (
            StatusCode::OK,
            [(CACHE_CONTROL, "no-store")],
            Json(RuntimeLlmConfigResponse {
                contract: 1,
                api_url: config.api_url,
                model: config.model,
                api_key: config.api_key,
            }),
        )
            .into_response(),
        Err(error) => auth_error_response(error),
    }
}

fn secrets_equal(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let status = readiness_status(state.readiness.as_ref()).await;
    if status == StatusCode::SERVICE_UNAVAILABLE {
        tracing::warn!("user-service readiness check failed");
    }
    let body = if status == StatusCode::OK {
        serde_json::json!({"status": "ready"})
    } else {
        serde_json::json!({"status": "not_ready"})
    };
    (status, Json(body)).into_response()
}

async fn readiness_status(probe: &dyn ReadinessProbe) -> StatusCode {
    if probe.is_ready().await {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

fn extract_user_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
}

#[cfg(test)]
mod readiness_tests {
    use super::*;
    use async_trait::async_trait;

    struct FixedProbe(bool);

    #[async_trait]
    impl ReadinessProbe for FixedProbe {
        async fn is_ready(&self) -> bool {
            self.0
        }
    }

    #[tokio::test]
    async fn readiness_status_fails_closed() {
        assert_eq!(readiness_status(&FixedProbe(true)).await, StatusCode::OK);
        assert_eq!(
            readiness_status(&FixedProbe(false)).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
