use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::application::handlers::AuthHandler;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<AuthHandler>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/refresh", post(refresh))
        .route("/auth/me", get(get_me))
        .route("/auth/logout", post(logout))
        .route("/health", get(health))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    email: String,
    password: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct RefreshRequest {
    refresh_token: String,
}

#[derive(Debug, Deserialize)]
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
    message: String,
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

fn error_response(status: StatusCode, msg: &str) -> impl IntoResponse {
    (status, Json(ApiError { error: ErrorDetail { message: msg.to_string() } }))
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> impl IntoResponse {
    match state.handler.register(&req.email, &req.password, req.name).await {
        Ok((user, access_token, refresh_token)) => (
            StatusCode::CREATED,
            Json(AuthResponse {
                user: user_dto(&user),
                access_token,
                refresh_token,
            }),
        ).into_response(),
        Err(e) => {
            let msg = e.to_string();
            let status = if msg.contains("already registered") {
                StatusCode::CONFLICT
            } else if msg.contains("Invalid email") || msg.contains("Password must") {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            error_response(status, &msg).into_response()
        }
    }
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<LoginRequest>,
) -> impl IntoResponse {
    match state.handler.login(&req.email, &req.password).await {
        Ok((user, access_token, refresh_token)) => (
            StatusCode::OK,
            Json(AuthResponse {
                user: user_dto(&user),
                access_token,
                refresh_token,
            }),
        ).into_response(),
        Err(e) => error_response(StatusCode::UNAUTHORIZED, &e.to_string()).into_response(),
    }
}

async fn refresh(
    State(state): State<AppState>,
    Json(req): Json<RefreshRequest>,
) -> impl IntoResponse {
    match state.handler.refresh(&req.refresh_token).await {
        Ok(access_token) => (StatusCode::OK, Json(TokenResponse { access_token })).into_response(),
        Err(e) => error_response(StatusCode::UNAUTHORIZED, &e.to_string()).into_response(),
    }
}

async fn get_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return error_response(StatusCode::UNAUTHORIZED, "Missing user ID").into_response(),
    };
    match state.handler.get_me(user_id).await {
        Ok(user) => (StatusCode::OK, Json(user_dto(&user))).into_response(),
        Err(e) => error_response(StatusCode::NOT_FOUND, &e.to_string()).into_response(),
    }
}

async fn logout(
    State(state): State<AppState>,
    Json(req): Json<LogoutRequest>,
) -> impl IntoResponse {
    match state.handler.logout(&req.refresh_token).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => error_response(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()).into_response(),
    }
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

fn extract_user_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
}
