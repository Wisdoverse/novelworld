use axum::{
    extract::{DefaultBodyLimit, Json, Path, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_TYPE},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::IntoResponse,
    routing::{get, post, put},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::application::handlers::{AuthError, AuthHandler, LlmUsageError, LlmUsageHandler};
use crate::domain::ports::ReadinessProbe;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<AuthHandler>,
    pub llm_usage_handler: Arc<LlmUsageHandler>,
    pub readiness: Arc<dyn ReadinessProbe>,
    pub internal_service_token: Arc<str>,
    pub metrics: llm_client::MetricsHandle,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/me", get(get_me).delete(delete_me))
        .route("/auth/logout", post(logout))
        .route("/setup/status", get(setup_status))
        .route("/setup/init", post(setup_init))
        .route("/settings/llm", get(get_llm_settings))
        .route("/settings/llm", put(update_llm_settings))
        .route("/settings/llm/usage", get(get_llm_usage))
        .route("/internal/runtime/llm", get(runtime_llm_config))
        .route(
            "/internal/privacy/users/{user_id}/export",
            get(export_account),
        )
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(state)
}

async fn export_account(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> axum::response::Response {
    if !internal_request_authorized(&state, &headers) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid internal service identity",
        )
        .into_response();
    }

    match state.handler.get_me(user_id).await {
        Ok(user) => {
            let record = profile_export_record(&user);
            match serde_json::to_string(&record) {
                Ok(line) => (
                    StatusCode::OK,
                    [
                        (CONTENT_TYPE, "application/x-ndjson"),
                        (CACHE_CONTROL, "no-store"),
                    ],
                    format!("{line}\n"),
                )
                    .into_response(),
                Err(error) => {
                    tracing::error!(?error, %user_id, "failed to serialize account export");
                    error_response(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "internal_error",
                        "Account export failed",
                    )
                    .into_response()
                }
            }
        }
        Err(error) => auth_error_response(error),
    }
}

fn profile_export_record(user: &crate::domain::entities::user::User) -> serde_json::Value {
    serde_json::json!({
        "type": "record",
        "service": "user",
        "kind": "profile",
        "data": {
            "id": user.id,
            "email": user.email,
            "name": user.name,
            "avatar_url": user.avatar_url,
            "role": user.role.as_str(),
            "email_verified": user.email_verified,
            "created_at": user.created_at,
            "updated_at": user.updated_at,
            "last_sign_in": user.last_sign_in,
        }
    })
}

async fn metrics(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(
            axum::http::header::CONTENT_TYPE,
            "text/plain; version=0.0.4",
        )],
        state.metrics.render(),
    )
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
    refresh_token: String,
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
}

#[derive(Serialize)]
struct RuntimeLlmConfigResponse {
    contract: u8,
    api_url: String,
    model: String,
    thinking_enabled: bool,
    api_key: String,
}

#[derive(Debug, Serialize)]
struct LlmSettingsResponse {
    provider: String,
    model: String,
    thinking_enabled: bool,
    api_key_configured: bool,
    scope: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateLlmSettingsRequest {
    provider: String,
    model: String,
    api_key: Option<String>,
    thinking_enabled: bool,
}

#[derive(Debug, Serialize)]
struct LlmUsageResponse {
    contract: u8,
    scope: &'static str,
    window_days: u16,
    tokens: LlmUsageTokensResponse,
    costs: LlmUsageCostsResponse,
    unpriced_tokens: String,
}

#[derive(Debug, Serialize)]
struct LlmUsageTokensResponse {
    input: String,
    cached_input: String,
    uncached_input: String,
    output: String,
    total: String,
}

#[derive(Debug, Serialize)]
struct LlmUsageCostsResponse {
    usd_micros: Option<String>,
    cny_micros: Option<String>,
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
    if matches!(&error, AuthError::Capacity) {
        let mut response = error_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "capacity_unavailable",
            "Authentication capacity is busy; retry the request",
        )
        .into_response();
        response
            .headers_mut()
            .insert("Retry-After", HeaderValue::from_static("1"));
        return response;
    }
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
        AuthError::LastAdministrator => (
            StatusCode::CONFLICT,
            "last_administrator",
            "The other users must delete their accounts before the only administrator can be deleted"
                .into(),
        ),
        AuthError::PrivacyCleanupUnavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "privacy_cleanup_unavailable",
            "Account deletion is temporarily unavailable; retry without signing out".into(),
        ),
        AuthError::Capacity => unreachable!("capacity errors return above"),
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

fn llm_usage_error_response(error: LlmUsageError) -> axum::response::Response {
    let (status, code, message): (StatusCode, &'static str, &'static str) = match error {
        LlmUsageError::PersonalKeyRequired => (
            StatusCode::FORBIDDEN,
            "personal_llm_key_required",
            "Configure a personal LLM API key to view its usage",
        ),
        LlmUsageError::Unavailable => (
            StatusCode::SERVICE_UNAVAILABLE,
            "llm_usage_unavailable",
            "LLM usage statistics are temporarily unavailable",
        ),
        LlmUsageError::NotFound => (StatusCode::NOT_FOUND, "not_found", "User not found"),
        LlmUsageError::Internal(error) => {
            tracing::error!(error = ?error, "LLM usage operation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "LLM usage operation failed",
            )
        }
    };
    error_response(status, code, message).into_response()
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
        Ok((access_token, refresh_token)) => (
            StatusCode::OK,
            Json(TokenResponse {
                access_token,
                refresh_token,
            }),
        )
            .into_response(),
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

async fn delete_me(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return auth_error_response(AuthError::InvalidCredentials),
    };
    match state.handler.delete_account(user_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
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
                configured: status.admin_configured,
                admin_configured: status.admin_configured,
                llm_configured: status.llm_configured,
                contract: 4,
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

async fn runtime_llm_config(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !internal_request_authorized(&state, &headers) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Invalid internal service identity",
        )
        .into_response();
    }

    let user_id = match extract_optional_user_id(&headers) {
        Ok(user_id) => user_id,
        Err(()) => {
            return error_response(
                StatusCode::BAD_REQUEST,
                "invalid_user_context",
                "X-User-Id must be a valid UUID when provided",
            )
            .into_response()
        }
    };

    match state.handler.runtime_llm_config_for(user_id).await {
        Ok(config) => (
            StatusCode::OK,
            [(CACHE_CONTROL, "no-store")],
            Json(RuntimeLlmConfigResponse {
                contract: 2,
                api_url: config.api_url,
                model: config.model,
                thinking_enabled: config.thinking_enabled,
                api_key: config.api_key,
            }),
        )
            .into_response(),
        Err(error) => auth_error_response(error),
    }
}

fn internal_request_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    internal_token_authorized(headers, state.internal_service_token.as_ref())
}

fn internal_token_authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("X-Internal-Service-Token")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| secrets_equal(value, expected))
}

async fn get_llm_settings(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return auth_error_response(AuthError::InvalidCredentials),
    };
    match state.handler.llm_settings(user_id).await {
        Ok(settings) => (
            StatusCode::OK,
            [(CACHE_CONTROL, "no-store")],
            Json(LlmSettingsResponse {
                provider: settings.config.provider,
                model: settings.config.model,
                thinking_enabled: settings.config.thinking_enabled,
                api_key_configured: settings.api_key_configured,
                scope: settings.scope.as_str(),
            }),
        )
            .into_response(),
        Err(error) => auth_error_response(error),
    }
}

async fn get_llm_usage(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return auth_error_response(AuthError::InvalidCredentials),
    };
    match state.llm_usage_handler.summary(user_id).await {
        Ok((summary, scope)) => (
            StatusCode::OK,
            [(CACHE_CONTROL, "no-store")],
            Json(LlmUsageResponse {
                contract: 1,
                scope: scope.as_str(),
                window_days: summary.window_days,
                tokens: LlmUsageTokensResponse {
                    input: summary.input_tokens().to_string(),
                    cached_input: summary.cached_input_tokens.to_string(),
                    uncached_input: summary.uncached_input_tokens.to_string(),
                    output: summary.output_tokens.to_string(),
                    total: summary.total_tokens().to_string(),
                },
                costs: LlmUsageCostsResponse {
                    usd_micros: summary.usd_micros.map(|value| value.to_string()),
                    cny_micros: summary.cny_micros.map(|value| value.to_string()),
                },
                unpriced_tokens: summary.unpriced_tokens.to_string(),
            }),
        )
            .into_response(),
        Err(error) => llm_usage_error_response(error),
    }
}

async fn update_llm_settings(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<UpdateLlmSettingsRequest>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return auth_error_response(AuthError::InvalidCredentials),
    };
    match state
        .handler
        .update_llm_settings(
            user_id,
            &request.provider,
            &request.model,
            request.api_key.as_deref(),
            request.thinking_enabled,
        )
        .await
    {
        Ok(settings) => Json(LlmSettingsResponse {
            provider: settings.config.provider,
            model: settings.config.model,
            thinking_enabled: settings.config.thinking_enabled,
            api_key_configured: settings.api_key_configured,
            scope: settings.scope.as_str(),
        })
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

fn extract_optional_user_id(headers: &HeaderMap) -> Result<Option<Uuid>, ()> {
    headers
        .get("X-User-Id")
        .map(|value| {
            value
                .to_str()
                .ok()
                .and_then(|value| Uuid::parse_str(value).ok())
                .ok_or(())
        })
        .transpose()
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

    #[test]
    fn setup_contract_separates_admin_bootstrap_from_llm_configuration() {
        let status = crate::application::handlers::SetupStatus {
            admin_configured: true,
            llm_configured: false,
        };
        assert!(status.admin_configured);
        assert!(serde_json::from_value::<SetupRequest>(serde_json::json!({
            "email": "admin@test.invalid",
            "password": "password123",
            "name": null,
            "provider": "openai"
        }))
        .is_err());
    }

    #[tokio::test]
    async fn missing_llm_settings_keep_the_setup_required_envelope() {
        let response = auth_error_response(AuthError::SetupRequired);
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"]["code"],
            "setup_required"
        );
    }

    #[test]
    fn internal_export_auth_rejects_missing_and_wrong_tokens() {
        let mut headers = HeaderMap::new();
        assert!(!internal_token_authorized(&headers, "expected-token"));
        headers.insert("X-Internal-Service-Token", "wrong-token".parse().unwrap());
        assert!(!internal_token_authorized(&headers, "expected-token"));
        headers.insert(
            "X-Internal-Service-Token",
            "expected-token".parse().unwrap(),
        );
        assert!(internal_token_authorized(&headers, "expected-token"));
    }

    #[test]
    fn optional_internal_user_context_is_strict_when_present() {
        let mut headers = HeaderMap::new();
        assert_eq!(extract_optional_user_id(&headers), Ok(None));
        let user_id = Uuid::new_v4();
        headers.insert("X-User-Id", user_id.to_string().parse().unwrap());
        assert_eq!(extract_optional_user_id(&headers), Ok(Some(user_id)));
        headers.insert("X-User-Id", "not-a-uuid".parse().unwrap());
        assert_eq!(extract_optional_user_id(&headers), Err(()));
    }

    #[test]
    fn profile_export_has_an_explicit_secret_free_shape() {
        let user = crate::domain::entities::user::User::new(
            "portable@test.invalid".into(),
            "SENTINEL_PASSWORD_HASH".into(),
            Some("Portable Reader".into()),
        );
        let serialized = serde_json::to_string(&profile_export_record(&user)).unwrap();
        assert!(serialized.contains("portable@test.invalid"));
        assert!(serialized.contains("Portable Reader"));
        assert!(!serialized.contains("SENTINEL_PASSWORD_HASH"));
        assert!(!serialized.contains("password_hash"));
    }
}
