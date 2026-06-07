use axum::{
    extract::{Path, State, Json},
    http::{StatusCode, HeaderMap},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::application::handlers::NarrativeCommandHandler;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<NarrativeCommandHandler>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/narrative/:novel_id/:chapter", get(get_branch_node))
        .route("/narrative/choose", post(submit_choice))
        .route("/narrative/:novel_id/world-state", get(get_world_state))
        .route("/health", get(health))
        .with_state(state)
}

// ─── Request/Response DTOs ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct SubmitChoiceRequest {
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub node_id: Uuid,
    pub choice_index: i32,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
}

// ─── Handlers ───────────────────────────────────────────────────────────────

/// GET /narrative/:novel_id/:chapter
async fn get_branch_node(
    State(state): State<AppState>,
    Path((novel_id, chapter)): Path<(Uuid, i32)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError { error: "Missing or invalid X-User-Id header".into() }),
            ).into_response();
        }
    };

    match state.handler.get_branch_node(novel_id, chapter, user_id).await {
        Ok(Some(node)) => (StatusCode::OK, Json(serde_json::json!(node))).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiError { error: "No branch node for this chapter".into() }),
        ).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError { error: e.to_string() }),
        ).into_response(),
    }
}

/// POST /narrative/choose
async fn submit_choice(
    State(state): State<AppState>,
    Json(req): Json<SubmitChoiceRequest>,
) -> impl IntoResponse {
    match state.handler.submit_choice(
        req.user_id,
        req.novel_id,
        req.node_id,
        req.choice_index,
    ).await {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))).into_response(),
        Err(e) => {
            let status = if e.to_string().contains("Invalid choice index")
                || e.to_string().contains("not found")
            {
                StatusCode::BAD_REQUEST
            } else {
                StatusCode::INTERNAL_SERVER_ERROR
            };
            (status, Json(ApiError { error: e.to_string() })).into_response()
        }
    }
}

/// GET /narrative/:novel_id/world-state
async fn get_world_state(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError { error: "Missing or invalid X-User-Id header".into() }),
            ).into_response();
        }
    };

    match state.handler.get_world_state(user_id, novel_id).await {
        Ok(ws) => (StatusCode::OK, Json(serde_json::json!(ws))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError { error: e.to_string() }),
        ).into_response(),
    }
}

/// GET /health
async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Extract user_id from X-User-Id header; returns None if missing or invalid.
fn extract_user_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
}
