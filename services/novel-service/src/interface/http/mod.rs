use axum::{
    extract::{Path, State, Json, Multipart},
    http::{StatusCode, HeaderMap},
    response::IntoResponse,
    routing::{get, post, delete},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use axum::routing::put;
use crate::application::commands::ImportNovelCommand;
use crate::application::handlers::NovelCommandHandler;
use crate::domain::repositories::{NovelRepository, ChapterRepository, CharacterRepository, ReadingProgressRepository};

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<NovelCommandHandler>,
    pub novel_repo: Arc<dyn NovelRepository>,
    pub chapter_repo: Arc<dyn ChapterRepository>,
    pub character_repo: Arc<dyn CharacterRepository>,
    pub progress_repo: Arc<dyn ReadingProgressRepository>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/novels", post(import_novel))
        .route("/novels", get(list_novels))
        .route("/novels/:id", get(get_novel))
        .route("/novels/:id", delete(delete_novel))
        .route("/novels/:id/chapters", get(list_chapters))
        .route("/novels/:id/chapters/:num", get(get_chapter))
        .route("/novels/:id/characters", get(list_characters))
        .route("/novels/:id/relationships", get(list_relationships))
        .route("/novels/:id/status", get(get_parse_status))
        .route("/progress/:novel_id", get(get_progress))
        .route("/progress/:novel_id", put(update_progress))
        .route("/progress/:novel_id/identity", put(set_identity))
        .route("/health", get(health))
        .with_state(state)
}

// ─── Request/Response DTOs ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ImportNovelRequest {
    pub title: String,
    pub author: Option<String>,
    pub content: Option<String>,
    pub deviation_mode: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ImportNovelResponse {
    pub novel_id: Uuid,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: String,
}

// ─── Handlers ─────────────────────────────────────────────────────────────────

async fn import_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ImportNovelRequest>,
) -> impl IntoResponse {
    let user_id = match headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
    {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError { error: "Missing or invalid X-User-Id header".into() }),
            ).into_response();
        }
    };

    let cmd = ImportNovelCommand {
        user_id,
        title: req.title,
        author: req.author,
        raw_content: req.content,
        file_key: None,
        deviation_mode: req.deviation_mode.as_deref().map(|m| match m {
            "creative" => crate::domain::value_objects::DeviationMode::Creative,
            "remix" => crate::domain::value_objects::DeviationMode::Remix,
            _ => crate::domain::value_objects::DeviationMode::Canon,
        }),
    };

    match state.handler.handle_import(cmd).await {
        Ok(novel_id) => (
            StatusCode::ACCEPTED,
            Json(ImportNovelResponse {
                novel_id,
                status: "parsing".into(),
                message: "Novel import started. Poll /novels/:id/status for progress.".into(),
            }),
        ).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError { error: e.to_string() }),
        ).into_response(),
    }
}

async fn list_novels(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = match headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
    {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError { error: "Missing or invalid X-User-Id header".into() }),
            ).into_response();
        }
    };
    match state.novel_repo.find_by_user(user_id).await {
        Ok(novels) => (StatusCode::OK, Json(novels)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn get_novel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.novel_repo.find_by_id(id).await {
        Ok(Some(novel)) => (StatusCode::OK, Json(novel)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(ApiError { error: "Novel not found".into() })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn delete_novel(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.novel_repo.delete(id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn list_chapters(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.chapter_repo.find_by_novel(novel_id).await {
        Ok(chapters) => (StatusCode::OK, Json(chapters)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn get_chapter(
    State(state): State<AppState>,
    Path((novel_id, num)): Path<(Uuid, i32)>,
) -> impl IntoResponse {
    match state.chapter_repo.find_by_number(novel_id, num).await {
        Ok(Some(ch)) => (StatusCode::OK, Json(ch)).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(ApiError { error: "Chapter not found".into() })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn list_characters(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.character_repo.find_by_novel(novel_id).await {
        Ok(chars) => (StatusCode::OK, Json(chars)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn list_relationships(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    match state.character_repo.find_relationships(novel_id).await {
        Ok(rels) => (StatusCode::OK, Json(rels)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn get_parse_status(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match state.novel_repo.find_by_id(id).await {
        Ok(Some(novel)) => (StatusCode::OK, Json(serde_json::json!({
            "novel_id": novel.id,
            "status": format!("{:?}", novel.status),
            "total_chapters": novel.total_chapters,
            "error": novel.parse_error,
        }))).into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, Json(ApiError { error: "Novel not found".into() })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

// ─── Progress Handlers ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct UpdateProgressRequest {
    current_chapter: i32,
}

#[derive(Debug, Deserialize)]
struct SetIdentityRequest {
    identity_type: String,
    identity_name: Option<String>,
    character_id: Option<Uuid>,
}

fn extract_user_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
}

async fn get_progress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return (StatusCode::UNAUTHORIZED, Json(ApiError { error: "Missing user ID".into() })).into_response(),
    };
    match state.progress_repo.get_or_create(user_id, novel_id).await {
        Ok(progress) => (StatusCode::OK, Json(progress)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn update_progress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
    Json(req): Json<UpdateProgressRequest>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return (StatusCode::UNAUTHORIZED, Json(ApiError { error: "Missing user ID".into() })).into_response(),
    };
    match state.progress_repo.update_chapter(user_id, novel_id, req.current_chapter).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn set_identity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
    Json(req): Json<SetIdentityRequest>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return (StatusCode::UNAUTHORIZED, Json(ApiError { error: "Missing user ID".into() })).into_response(),
    };
    match state.progress_repo.set_identity(
        user_id, novel_id,
        &req.identity_type,
        req.identity_name.as_deref(),
        req.character_id,
    ).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(ApiError { error: e.to_string() })).into_response(),
    }
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}
