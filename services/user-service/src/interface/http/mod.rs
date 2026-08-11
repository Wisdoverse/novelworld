use axum::{
    extract::{DefaultBodyLimit, Json, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{de::IgnoredAny, Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::application::handlers::{AuthError, AuthHandler};
use crate::domain::ports::ReadinessProbe;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<AuthHandler>,
    pub readiness: Arc<dyn ReadinessProbe>,
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
        .route("/setup/test-llm", post(deprecated_setup_llm_test))
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
    contract: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SetupRequest {
    email: String,
    password: String,
    name: Option<String>,
    // Rolling compatibility only: old setup clients sent these fields even
    // though the gateway never persisted them.
    #[serde(default, rename = "provider")]
    _legacy_provider: Option<IgnoredAny>,
    #[serde(default, rename = "api_key")]
    _legacy_api_key: Option<IgnoredAny>,
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
    match state.handler.is_configured().await {
        Ok(configured) => (
            StatusCode::OK,
            Json(SetupStatus {
                configured,
                contract: 2,
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
        .setup(&req.email, &req.password, req.name)
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

async fn deprecated_setup_llm_test() -> impl IntoResponse {
    error_response(
        StatusCode::UPGRADE_REQUIRED,
        "client_upgrade_required",
        "Refresh the setup page; model credentials are configured by the operator",
    )
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
