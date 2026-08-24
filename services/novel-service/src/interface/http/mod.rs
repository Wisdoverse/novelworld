use axum::{
    body::Body,
    extract::{multipart::MultipartRejection, DefaultBodyLimit, Json, Multipart, Path, State},
    http::{header::CACHE_CONTROL, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::application::commands::ImportNovelCommand;
use crate::application::handlers::{
    GameRuleTemplateRequest, GameRuleTemplateRequestError, ImportBudgetExceeded,
    ImportCapacityUnavailable, ImportRetryConflict, NovelCommandHandler, ReadingProgressError,
    ReadingProgressHandler, ShelfMutationError, SourceFileStorageUnavailable,
    TranslateChapterHandler, TranslationError,
};
use crate::domain::entities::novel::Novel;
use crate::domain::ports::{
    AccountExportPort, DocumentExtractionError, DocumentTextExtractor, ReadinessProbe,
};
use crate::domain::repositories::{
    CanonStoryModelRepository, ChapterRepository, CharacterRepository, NovelRepository,
};
use crate::domain::services::canon_story_context::{
    build_canon_context, build_world_entry_context, original_player_name_available,
};
use crate::domain::value_objects::DeviationMode;
use axum::routing::put;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<NovelCommandHandler>,
    pub novel_repo: Arc<dyn NovelRepository>,
    pub chapter_repo: Arc<dyn ChapterRepository>,
    pub character_repo: Arc<dyn CharacterRepository>,
    pub canon_repo: Arc<dyn CanonStoryModelRepository>,
    pub progress_handler: Arc<ReadingProgressHandler>,
    pub translation_handler: Arc<TranslateChapterHandler>,
    pub document_extractor: Arc<dyn DocumentTextExtractor>,
    pub document_parse_permits: Arc<Semaphore>,
    pub account_export: Arc<dyn AccountExportPort>,
    pub internal_service_token: Arc<str>,
    pub readiness: Arc<dyn ReadinessProbe>,
    pub source_storage_readiness: Option<Arc<dyn ReadinessProbe>>,
    pub metrics: llm_client::MetricsHandle,
}

pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

fn routes() -> Router<AppState> {
    Router::new()
        .route("/novels", post(import_novel))
        .route("/novels/upload", post(upload_novel))
        .route("/novels", get(list_novels))
        .route("/novels/catalog", get(list_catalog))
        .route("/novels/{id}", get(get_novel))
        .route("/novels/{id}", delete(delete_novel))
        .route("/novels/{id}/shelf", post(attach_novel))
        .route("/novels/{id}/retry", post(retry_novel))
        .route("/novels/{id}/chapters", get(list_chapters))
        .route("/novels/{id}/chapters/{num}", get(get_chapter))
        .route(
            "/novels/{id}/chapters/{num}/translation",
            post(translate_chapter_text),
        )
        .route("/novels/{id}/lore/search", post(search_lore))
        .route("/novels/{id}/characters", get(list_characters))
        .route("/characters/{id}", get(get_character_by_id))
        .route(
            "/internal/novels/{id}/canon-context/{chapter}",
            get(get_canon_context),
        )
        .route(
            "/internal/novels/{id}/player-entry",
            post(get_player_entry_context),
        )
        .route(
            "/internal/novels/{id}/world-entry/{checkpoint}",
            get(get_world_entry_context),
        )
        .route(
            "/internal/novels/{id}/game-rules",
            post(request_game_rule_template),
        )
        .route(
            "/internal/novels/{id}/game-rules/{model_version}",
            get(get_game_rule_template),
        )
        .route(
            "/internal/privacy/users/{user_id}/export",
            get(export_account),
        )
        .route("/novels/{id}/relationships", get(list_relationships))
        .route("/novels/{id}/status", get(get_parse_status))
        .route("/progress/{novel_id}", get(get_progress))
        .route("/progress/{novel_id}", put(update_progress))
        .route("/progress/{novel_id}/identity", put(set_identity))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_SIZE))
}

async fn request_game_rule_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "Invalid internal service identity",
        );
    }
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return api_error(StatusCode::UNAUTHORIZED, "Missing user ID"),
    };
    let progress = match state.progress_handler.get(user_id, novel_id).await {
        Ok(progress) => progress,
        Err(error) => return progress_error_response(error),
    };
    match state
        .handler
        .request_game_rule_template(user_id, novel_id)
        .await
    {
        Ok(GameRuleTemplateRequest::Ready(template)) => {
            match template.visible_at(progress.current_chapter) {
                Some(visible) => (StatusCode::OK, Json(visible)).into_response(),
                None => coded_api_error(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "game_rules_unavailable_at_progress",
                    "Game rules are not yet available at current reading progress",
                ),
            }
        }
        Ok(GameRuleTemplateRequest::InProgress {
            retry_after_seconds,
        }) => {
            let mut response = coded_api_error(
                StatusCode::CONFLICT,
                "game_rule_generation_in_progress",
                "Game rule template generation is in progress",
            );
            if let Ok(value) = HeaderValue::from_str(&retry_after_seconds.to_string()) {
                response.headers_mut().insert("retry-after", value);
            }
            response
        }
        Err(GameRuleTemplateRequestError::NovelNotFound) => {
            coded_api_error(StatusCode::NOT_FOUND, "not_found", "Novel not found")
        }
        Err(GameRuleTemplateRequestError::CanonUnavailable) => coded_api_error(
            StatusCode::CONFLICT,
            "canon_unavailable",
            "Canonical story model is not ready",
        ),
        Err(GameRuleTemplateRequestError::BudgetExhausted) => coded_api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "game_rule_generation_exhausted",
            "Game rule generation budget is exhausted",
        ),
        Err(error) => {
            tracing::error!(%error, %novel_id, "game rule template request failed");
            coded_api_error(
                StatusCode::BAD_GATEWAY,
                "game_rule_generation_failed",
                "Game rule generation failed",
            )
        }
    }
}

async fn get_game_rule_template(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((novel_id, model_version)): Path<(Uuid, i32)>,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "Invalid internal service identity",
        );
    }
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return api_error(StatusCode::UNAUTHORIZED, "Missing user ID"),
    };
    if model_version < 1 {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Canonical model version must be at least 1",
        );
    }
    let progress = match state.progress_handler.get(user_id, novel_id).await {
        Ok(progress) => progress,
        Err(error) => return progress_error_response(error),
    };
    match state
        .canon_repo
        .find_game_rule_template(novel_id, model_version)
        .await
    {
        Ok(Some(template)) => match template.visible_at(progress.current_chapter) {
            Some(visible) => (StatusCode::OK, Json(visible)).into_response(),
            None => api_error(
                StatusCode::NOT_FOUND,
                "Game rule template is unavailable at current reading progress",
            ),
        },
        Ok(None) => api_error(StatusCode::NOT_FOUND, "Game rule template not found"),
        Err(error) => {
            tracing::error!(%error, %novel_id, model_version, "failed to load game rules");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

async fn export_account(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                error: "Invalid internal service identity".into(),
            }),
        )
            .into_response();
    }

    let stream = state.account_export.export_user(user_id).map(|result| {
        let record = result.map_err(std::io::Error::other)?;
        let mut line = serde_json::to_vec(&serde_json::json!({
            "type": "record",
            "service": "novel",
            "kind": record.kind,
            "data": record.data,
        }))
        .map_err(std::io::Error::other)?;
        line.push(b'\n');
        Ok::<_, std::io::Error>(line)
    });

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/x-ndjson")
        .header(CACHE_CONTROL, "no-store")
        .body(Body::from_stream(stream))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
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

async fn get_canon_context(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((novel_id, requested_chapter)): Path<(Uuid, i32)>,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "Invalid internal service identity",
        );
    }
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "Missing user ID".into(),
                }),
            )
                .into_response()
        }
    };
    if requested_chapter < 1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(ApiError {
                error: "chapter must be at least 1".into(),
            }),
        )
            .into_response();
    }
    let progress = match state.progress_handler.get(user_id, novel_id).await {
        Ok(progress) => progress,
        Err(error) => return progress_error_response(error),
    };
    let checkpoint = requested_chapter.min(progress.current_chapter);
    let model = match state.canon_repo.find_latest(novel_id).await {
        Ok(Some(model)) => model,
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "Canon context not found".into(),
                }),
            )
                .into_response()
        }
        Err(error) => {
            tracing::error!(%error, %novel_id, "failed to load canon context");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "Internal server error".into(),
                }),
            )
                .into_response();
        }
    };
    let characters = match state.character_repo.find_by_novel(novel_id).await {
        Ok(characters) => characters,
        Err(error) => {
            tracing::error!(%error, %novel_id, "failed to load canon characters");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "Internal server error".into(),
                }),
            )
                .into_response();
        }
    };
    match build_canon_context(&model, &characters, checkpoint) {
        Ok(context) => (StatusCode::OK, Json(context)).into_response(),
        Err(error) => {
            tracing::error!(%error, %novel_id, checkpoint, "invalid canon context");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "Internal server error".into(),
                }),
            )
                .into_response()
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlayerEntryContextRequest {
    checkpoint_chapter: Option<i32>,
    proposed_name: Option<String>,
}

#[derive(Debug, Serialize)]
struct PlayerEntryContextResponse {
    checkpoint_chapter: i32,
    name_available: bool,
    locations: Vec<crate::domain::services::canon_story_context::CanonEntityRef>,
}

async fn get_player_entry_context(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
    Json(req): Json<PlayerEntryContextRequest>,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "Invalid internal service identity",
        );
    }
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return api_error(StatusCode::UNAUTHORIZED, "Missing user ID"),
    };
    if req.proposed_name.as_ref().is_some_and(|name| {
        name.trim() != name
            || name.is_empty()
            || name.chars().count() > 100
            || name.chars().any(char::is_control)
    }) {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Player name must contain 1-100 trimmed printable characters",
        );
    }
    let progress = match state.progress_handler.get(user_id, novel_id).await {
        Ok(progress) => progress,
        Err(error) => return progress_error_response(error),
    };
    let checkpoint = req.checkpoint_chapter.unwrap_or(progress.current_chapter);
    if checkpoint < 1 || checkpoint > progress.current_chapter {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Player checkpoint must be within unlocked reading progress",
        );
    }
    let model = match state.canon_repo.find_latest(novel_id).await {
        Ok(Some(model)) => model,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "Canon context not found"),
        Err(error) => {
            tracing::error!(%error, %novel_id, "failed to load player entry canon");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let characters = match state.character_repo.find_by_novel(novel_id).await {
        Ok(characters) => characters,
        Err(error) => {
            tracing::error!(%error, %novel_id, "failed to load player entry characters");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let context = match build_canon_context(&model, &characters, checkpoint) {
        Ok(context) => context,
        Err(error) => {
            tracing::error!(%error, %novel_id, "invalid player entry context");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let name_available = req
        .proposed_name
        .as_deref()
        .is_none_or(|name| original_player_name_available(name, &characters));
    (
        StatusCode::OK,
        Json(PlayerEntryContextResponse {
            checkpoint_chapter: context.checkpoint_chapter,
            name_available,
            locations: context.locations,
        }),
    )
        .into_response()
}

async fn get_world_entry_context(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((novel_id, checkpoint)): Path<(Uuid, i32)>,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return api_error(
            StatusCode::UNAUTHORIZED,
            "Invalid internal service identity",
        );
    }
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return api_error(StatusCode::UNAUTHORIZED, "Missing user ID"),
    };
    let progress = match state.progress_handler.get(user_id, novel_id).await {
        Ok(progress) => progress,
        Err(error) => return progress_error_response(error),
    };
    if checkpoint < 1 || checkpoint > progress.current_chapter {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "World checkpoint must be within unlocked reading progress",
        );
    }
    let model = match state.canon_repo.find_latest(novel_id).await {
        Ok(Some(model)) => model,
        Ok(None) => return api_error(StatusCode::NOT_FOUND, "Canon context not found"),
        Err(error) => {
            tracing::error!(%error, %novel_id, "failed to load world-entry canon");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    let characters = match state.character_repo.find_by_novel(novel_id).await {
        Ok(characters) => characters,
        Err(error) => {
            tracing::error!(%error, %novel_id, "failed to load world-entry characters");
            return api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error");
        }
    };
    match build_world_entry_context(&model, &characters, checkpoint, progress.current_chapter) {
        Ok(context) => (StatusCode::OK, Json(context)).into_response(),
        Err(error) => {
            tracing::error!(%error, %novel_id, checkpoint, "invalid world-entry context");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

// ─── Request/Response DTOs ────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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

#[derive(Debug, Serialize)]
struct CodedApiError {
    error: CodedApiErrorDetail,
}

#[derive(Debug, Serialize)]
struct CodedApiErrorDetail {
    code: &'static str,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LoreSearchRequest {
    query: String,
    max_chapter: i32,
    limit: Option<usize>,
}

#[derive(Debug, Serialize)]
struct LoreSearchResponse {
    excerpts: Vec<crate::domain::repositories::LoreExcerpt>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TranslationRequest {
    content: String,
}

#[derive(Debug, Serialize)]
struct TranslationResponse {
    content: String,
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
                Json(ApiError {
                    error: "Missing or invalid X-User-Id header".into(),
                }),
            )
                .into_response();
        }
    };

    let (title, author, deviation_mode) =
        match validate_metadata(req.title, req.author, req.deviation_mode) {
            Ok(metadata) => metadata,
            Err((status, message)) => return api_error(status, message),
        };
    let content = match req.content {
        Some(content) if !content.trim().is_empty() && content.len() <= MAX_PASTE_SIZE => {
            Some(content)
        }
        Some(content) if content.len() > MAX_PASTE_SIZE => {
            return api_error(StatusCode::PAYLOAD_TOO_LARGE, "Pasted text exceeds 5 MiB")
        }
        _ => {
            return api_error(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Novel content is required",
            )
        }
    };

    let cmd = ImportNovelCommand {
        user_id,
        title,
        author,
        raw_content: content,
        source_bytes: None,
        deviation_mode,
    };

    match state.handler.handle_import(cmd).await {
        Ok(novel_id) => (
            StatusCode::ACCEPTED,
            Json(ImportNovelResponse {
                novel_id,
                status: "parsing".into(),
                message: "Novel import started. Poll /novels/:id/status for progress.".into(),
            }),
        )
            .into_response(),
        Err(error) => import_error_response(error),
    }
}

const MAX_PASTE_SIZE: usize = 5 * 1024 * 1024;
const MAX_FILE_SIZE: usize = 20 * 1024 * 1024;
const MAX_REQUEST_SIZE: usize = MAX_FILE_SIZE + 1024 * 1024;
const MAX_TITLE_CHARS: usize = 500;
const MAX_AUTHOR_CHARS: usize = 200;
type ValidatedMetadata = (
    String,
    Option<String>,
    Option<crate::domain::value_objects::DeviationMode>,
);

fn api_error(status: StatusCode, message: impl Into<String>) -> Response {
    (
        status,
        Json(ApiError {
            error: message.into(),
        }),
    )
        .into_response()
}

fn coded_api_error(status: StatusCode, code: &'static str, message: impl Into<String>) -> Response {
    (
        status,
        Json(CodedApiError {
            error: CodedApiErrorDetail {
                code,
                message: message.into(),
            },
        }),
    )
        .into_response()
}

fn import_error_response(error: anyhow::Error) -> Response {
    if error.downcast_ref::<ImportCapacityUnavailable>().is_some() {
        let mut response = api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Novel import capacity is busy; retry the request",
        );
        response
            .headers_mut()
            .insert("Retry-After", HeaderValue::from_static("1"));
        return response;
    }
    if error.downcast_ref::<ImportBudgetExceeded>().is_some() {
        return api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Novel exceeds the supported processing budget",
        );
    }
    if error
        .downcast_ref::<SourceFileStorageUnavailable>()
        .is_some()
    {
        let mut response = api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Source file storage is unavailable; retry the upload",
        );
        response
            .headers_mut()
            .insert("Retry-After", HeaderValue::from_static("5"));
        return response;
    }
    tracing::error!(%error, "novel import request failed");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

fn retry_error_response(error: anyhow::Error, novel_id: Uuid) -> Response {
    if error.downcast_ref::<ImportCapacityUnavailable>().is_some()
        || error.downcast_ref::<ImportBudgetExceeded>().is_some()
    {
        return import_error_response(error);
    }
    if let Some(conflict) = error.downcast_ref::<ImportRetryConflict>() {
        return api_error(StatusCode::CONFLICT, conflict.0);
    }
    tracing::error!(%error, %novel_id, "novel import retry failed");
    api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
}

fn validate_metadata(
    title: String,
    author: Option<String>,
    deviation_mode: Option<String>,
) -> Result<ValidatedMetadata, (StatusCode, &'static str)> {
    let title = title.trim().to_string();
    if title.is_empty()
        || title.chars().count() > MAX_TITLE_CHARS
        || title.chars().any(char::is_control)
    {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "Title must contain 1-500 printable characters",
        ));
    }
    let author = author
        .map(|author| author.trim().to_string())
        .filter(|author| !author.is_empty());
    if author.as_ref().is_some_and(|author| {
        author.chars().count() > MAX_AUTHOR_CHARS || author.chars().any(char::is_control)
    }) {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            "Author must contain at most 200 printable characters",
        ));
    }
    let mode = match deviation_mode.as_deref().unwrap_or("canon") {
        "canon" => crate::domain::value_objects::DeviationMode::Canon,
        "creative" => crate::domain::value_objects::DeviationMode::Creative,
        "remix" => crate::domain::value_objects::DeviationMode::Remix,
        _ => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                "Deviation mode must be canon, creative, or remix",
            ))
        }
    };
    Ok((title, author, Some(mode)))
}

fn document_error_response(error: DocumentExtractionError) -> Response {
    let status = match &error {
        DocumentExtractionError::UnsupportedType => StatusCode::UNSUPPORTED_MEDIA_TYPE,
        DocumentExtractionError::UploadTooLarge { .. }
        | DocumentExtractionError::ExtractedTextTooLarge { .. } => StatusCode::PAYLOAD_TOO_LARGE,
        DocumentExtractionError::InvalidTextEncoding
        | DocumentExtractionError::InvalidEpub(_)
        | DocumentExtractionError::InvalidPdf(_)
        | DocumentExtractionError::EmptyDocument => StatusCode::UNPROCESSABLE_ENTITY,
    };
    api_error(status, error.to_string())
}

/// POST /novels/upload -- multipart file upload (TXT, EPUB, or PDF).
async fn upload_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    multipart: Result<Multipart, MultipartRejection>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "Missing or invalid X-User-Id header".into(),
                }),
            )
                .into_response();
        }
    };

    let mut multipart = match multipart {
        Ok(multipart) => multipart,
        Err(error) => {
            tracing::warn!(error = ?error, "multipart upload rejected");
            return api_error(StatusCode::BAD_REQUEST, "Invalid multipart upload");
        }
    };

    let mut title: Option<String> = None;
    let mut author: Option<String> = None;
    let mut content: Option<String> = None;
    let mut source_bytes = None;
    let mut file_seen = false;
    let mut deviation_mode_str: Option<String> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(error) => {
                tracing::warn!(error = ?error, "invalid multipart upload");
                return api_error(StatusCode::BAD_REQUEST, "Invalid multipart upload");
            }
        };
        let name = field.name().unwrap_or("").to_string();
        match name.as_str() {
            "title" => {
                title = match field.text().await {
                    Ok(value) => Some(value),
                    Err(_) => return api_error(StatusCode::BAD_REQUEST, "Invalid title field"),
                };
            }
            "author" => {
                author = match field.text().await {
                    Ok(value) => Some(value),
                    Err(_) => return api_error(StatusCode::BAD_REQUEST, "Invalid author field"),
                };
            }
            "deviation_mode" => {
                deviation_mode_str = match field.text().await {
                    Ok(value) => Some(value),
                    Err(_) => {
                        return api_error(StatusCode::BAD_REQUEST, "Invalid deviation mode field")
                    }
                };
            }
            "file" => {
                if file_seen {
                    return api_error(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "Exactly one file must be uploaded",
                    );
                }
                file_seen = true;
                let file_name = field.file_name().map(str::to_owned);
                let content_type = field.content_type().map(str::to_owned);
                let data = match field.bytes().await {
                    Ok(d) => d,
                    Err(_) => return api_error(StatusCode::BAD_REQUEST, "Invalid file upload"),
                };
                let permit = match state.document_parse_permits.clone().try_acquire_owned() {
                    Ok(permit) => permit,
                    Err(_) => {
                        let mut response = api_error(
                            StatusCode::SERVICE_UNAVAILABLE,
                            "Document parser capacity is busy; retry the request",
                        );
                        response
                            .headers_mut()
                            .insert("Retry-After", HeaderValue::from_static("1"));
                        return response;
                    }
                };
                let extractor = state.document_extractor.clone();
                let extracted = tokio::task::spawn_blocking(move || {
                    let _permit = permit;
                    extractor
                        .extract_text(file_name.as_deref(), content_type.as_deref(), &data)
                        .map(|text| (text, data))
                })
                .await;
                match extracted {
                    Err(error) => {
                        tracing::error!(%error, "document parser task failed");
                        return api_error(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Internal server error",
                        );
                    }
                    Ok(Ok((text, data))) => {
                        content = Some(text);
                        source_bytes = Some(data);
                    }
                    Ok(Err(error)) => return document_error_response(error),
                }
            }
            _ => {
                // Ignore unknown fields
            }
        }
    }

    // Validate required fields
    let title = match title {
        Some(title) => title,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: "Missing required field: title".into(),
                }),
            )
                .into_response();
        }
    };

    let content = match content {
        Some(c) if !c.is_empty() => c,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ApiError {
                    error: "Missing or empty file upload".into(),
                }),
            )
                .into_response();
        }
    };

    let (title, author, deviation_mode) = match validate_metadata(title, author, deviation_mode_str)
    {
        Ok(metadata) => metadata,
        Err((status, message)) => return api_error(status, message),
    };

    let cmd = ImportNovelCommand {
        user_id,
        title,
        author,
        raw_content: Some(content),
        source_bytes,
        deviation_mode,
    };

    match state.handler.handle_import(cmd).await {
        Ok(novel_id) => (
            StatusCode::ACCEPTED,
            Json(ImportNovelResponse {
                novel_id,
                status: "parsing".into(),
                message:
                    "Novel file uploaded and import started. Poll /novels/:id/status for progress."
                        .into(),
            }),
        )
            .into_response(),
        Err(error) => import_error_response(error),
    }
}

async fn list_novels(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user_id = match headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| Uuid::parse_str(v).ok())
    {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "Missing or invalid X-User-Id header".into(),
                }),
            )
                .into_response();
        }
    };
    match state.novel_repo.find_by_user(user_id).await {
        Ok(novels) => (StatusCode::OK, Json(novels)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn list_catalog(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    match state.novel_repo.find_available_to_user(user_id).await {
        Ok(novels) => (StatusCode::OK, Json(novels)).into_response(),
        Err(error) => {
            tracing::error!(error = ?error, %user_id, "shared novel catalog lookup failed");
            api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Shared library lookup failed",
            )
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttachNovelRequest {
    deviation_mode: Option<String>,
}

async fn attach_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(request): Json<AttachNovelRequest>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    let mode = match request.deviation_mode.as_deref().unwrap_or("canon") {
        "canon" => DeviationMode::Canon,
        "creative" => DeviationMode::Creative,
        "remix" => DeviationMode::Remix,
        _ => return api_error(StatusCode::UNPROCESSABLE_ENTITY, "Invalid deviation_mode"),
    };
    match state.handler.attach_shared_novel(user_id, id, mode).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ShelfMutationError::NotFound) => api_error(StatusCode::NOT_FOUND, "Novel not found"),
        Err(ShelfMutationError::PrivacyCleanup(error)) => privacy_cleanup_error(error),
        Err(ShelfMutationError::Repository(error)) => {
            tracing::error!(error = ?error, %user_id, novel_id = %id, "shelf attachment failed");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "Shelf attachment failed")
        }
    }
}

async fn get_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match owned_novel(&state, &headers, id).await {
        Ok(novel) => (StatusCode::OK, Json(novel)).into_response(),
        Err(response) => *response,
    }
}

async fn delete_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    match state.handler.remove_from_shelf(user_id, id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(ShelfMutationError::NotFound) => (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "Novel not found".into(),
            }),
        )
            .into_response(),
        Err(ShelfMutationError::PrivacyCleanup(error)) => privacy_cleanup_error(error),
        Err(ShelfMutationError::Repository(error)) => {
            tracing::error!(error = ?error, %user_id, novel_id = %id, "shelf removal failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    error: "Shelf removal failed".into(),
                }),
            )
                .into_response()
        }
    }
}

fn privacy_cleanup_error(error: anyhow::Error) -> Response {
    tracing::warn!(error = ?error, "shelf cache projection update failed");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(ApiError {
            error: "Shelf update is temporarily unavailable; retry the request".into(),
        }),
    )
        .into_response()
}

async fn retry_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    if let Err(response) = owned_novel(&state, &headers, id).await {
        return *response;
    }
    match state.handler.retry_import(user_id, id).await {
        Ok(()) => (
            StatusCode::ACCEPTED,
            Json(ImportNovelResponse {
                novel_id: id,
                status: "parsing".into(),
                message: "Novel import retry started.".into(),
            }),
        )
            .into_response(),
        Err(error) => retry_error_response(error, id),
    }
}

async fn list_chapters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(response) = owned_novel(&state, &headers, novel_id).await {
        return *response;
    }
    match state.chapter_repo.find_by_novel(novel_id).await {
        Ok(chapters) => (StatusCode::OK, Json(chapters)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn get_chapter(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((novel_id, num)): Path<(Uuid, i32)>,
) -> impl IntoResponse {
    if let Err(response) = owned_novel(&state, &headers, novel_id).await {
        return *response;
    }
    match state.chapter_repo.find_by_number(novel_id, num).await {
        Ok(Some(ch)) => (StatusCode::OK, Json(ch)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "Chapter not found".into(),
            }),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn translate_chapter_text(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((novel_id, chapter_number)): Path<(Uuid, i32)>,
    Json(request): Json<TranslationRequest>,
) -> Response {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => return api_error(StatusCode::UNAUTHORIZED, "Missing user ID"),
    };
    if let Err(response) = owned_novel(&state, &headers, novel_id).await {
        return *response;
    }
    match state
        .translation_handler
        .translate(user_id, novel_id, chapter_number, &request.content)
        .await
    {
        Ok(content) => (StatusCode::OK, Json(TranslationResponse { content })).into_response(),
        Err(TranslationError::Validation) => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Translation source must contain 1-48000 bytes",
        ),
        Err(TranslationError::Capacity) => {
            let mut response = api_error(
                StatusCode::TOO_MANY_REQUESTS,
                "Too many translations are running",
            );
            response
                .headers_mut()
                .insert("Retry-After", HeaderValue::from_static("5"));
            response
        }
        Err(TranslationError::InProgress {
            retry_after_seconds,
        }) => {
            let mut response =
                api_error(StatusCode::CONFLICT, "Translation is already in progress");
            if let Ok(value) = HeaderValue::try_from(retry_after_seconds.to_string()) {
                response.headers_mut().insert("Retry-After", value);
            }
            response
        }
        Err(TranslationError::ChapterNotFound) => {
            api_error(StatusCode::NOT_FOUND, "Chapter not found")
        }
        Err(TranslationError::SourceMismatch) => api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Translation source must be visible text from the requested chapter",
        ),
        Err(TranslationError::Timeout) => {
            api_error(StatusCode::GATEWAY_TIMEOUT, "Translation timed out")
        }
        Err(TranslationError::Provider(error)) => {
            tracing::warn!(error = ?error, %novel_id, "chapter translation failed");
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Translation is temporarily unavailable",
            )
        }
        Err(TranslationError::Repository(error)) => {
            tracing::error!(error = ?error, %novel_id, chapter_number, "translation chapter lookup failed");
            api_error(StatusCode::INTERNAL_SERVER_ERROR, "Internal server error")
        }
    }
}

async fn list_characters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    match state
        .progress_handler
        .list_available_characters(user_id, novel_id)
        .await
    {
        Ok(chars) => (StatusCode::OK, Json(chars)).into_response(),
        Err(error) => progress_error_response(error),
    }
}

async fn get_character_by_id(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(user_id) => user_id,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };
    match state
        .progress_handler
        .get_available_character(user_id, id)
        .await
    {
        Ok(character) => (StatusCode::OK, Json(character)).into_response(),
        Err(error) => progress_error_response(error),
    }
}

async fn list_relationships(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(response) = owned_novel(&state, &headers, novel_id).await {
        return *response;
    }
    match state.character_repo.find_relationships(novel_id).await {
        Ok(rels) => (StatusCode::OK, Json(rels)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn get_parse_status(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match owned_novel(&state, &headers, id).await {
        Ok(novel) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "novel_id": novel.id,
                "status": novel.status.to_str(),
                "total_chapters": novel.total_chapters,
                "error": novel.parse_error,
            })),
        )
            .into_response(),
        Err(response) => *response,
    }
}

// ─── Progress Handlers ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UpdateProgressRequest {
    current_chapter: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
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

fn internal_request_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    internal_token_authorized(headers, state.internal_service_token.as_ref())
}

fn internal_token_authorized(headers: &HeaderMap, expected: &str) -> bool {
    headers
        .get("X-Internal-Service-Token")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| secrets_equal(value, expected))
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

async fn owned_novel(
    state: &AppState,
    headers: &HeaderMap,
    novel_id: Uuid,
) -> Result<Novel, Box<Response>> {
    let user_id = extract_user_id(headers).ok_or_else(|| {
        Box::new(
            (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "Missing or invalid X-User-Id header".into(),
                }),
            )
                .into_response(),
        )
    })?;

    match state.novel_repo.find_for_user(user_id, novel_id).await {
        Ok(Some(novel)) => Ok(novel),
        Ok(_) => Err(Box::new(
            (
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    error: "Novel not found".into(),
                }),
            )
                .into_response(),
        )),
        Err(error) => {
            tracing::error!(%error, %novel_id, "owned novel lookup failed");
            Err(Box::new(api_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal server error",
            )))
        }
    }
}

fn progress_error_response(error: ReadingProgressError) -> Response {
    let (status, code, message) = match error {
        ReadingProgressError::NotFound => {
            (StatusCode::NOT_FOUND, "not_found", "Novel not found".into())
        }
        ReadingProgressError::CharacterNotFound => (
            StatusCode::NOT_FOUND,
            "not_found",
            "Character not found".into(),
        ),
        ReadingProgressError::Validation(message) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            message,
        ),
        ReadingProgressError::Internal(error) => {
            tracing::error!(error = ?error, "reading progress operation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Reading progress operation failed".into(),
            )
        }
    };
    (
        status,
        Json(serde_json::json!({
            "error": {"code": code, "message": message}
        })),
    )
        .into_response()
}

async fn get_progress(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "Missing user ID".into(),
                }),
            )
                .into_response()
        }
    };
    match state.progress_handler.get(user_id, novel_id).await {
        Ok(progress) => (StatusCode::OK, Json(progress)).into_response(),
        Err(error) => progress_error_response(error),
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
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "Missing user ID".into(),
                }),
            )
                .into_response()
        }
    };
    match state
        .progress_handler
        .update_chapter(user_id, novel_id, req.current_chapter)
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => progress_error_response(error),
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
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "Missing user ID".into(),
                }),
            )
                .into_response()
        }
    };
    match state
        .progress_handler
        .set_identity(
            user_id,
            novel_id,
            &req.identity_type,
            req.identity_name,
            req.character_id,
        )
        .await
    {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => progress_error_response(error),
    }
}

async fn search_lore(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
    Json(req): Json<LoreSearchRequest>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ApiError {
                    error: "Missing user ID".into(),
                }),
            )
                .into_response()
        }
    };

    match state
        .progress_handler
        .search_lore(
            user_id,
            novel_id,
            req.max_chapter,
            &req.query,
            req.limit.unwrap_or(3),
        )
        .await
    {
        Ok(excerpts) => (StatusCode::OK, Json(LoreSearchResponse { excerpts })).into_response(),
        Err(error) => progress_error_response(error),
    }
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let status = readiness_status(
        state.readiness.as_ref(),
        state.source_storage_readiness.as_deref(),
    )
    .await;
    if status == StatusCode::SERVICE_UNAVAILABLE {
        tracing::warn!("novel-service readiness check failed");
    }
    let body = if status == StatusCode::OK {
        serde_json::json!({"status": "ready"})
    } else {
        serde_json::json!({"status": "not_ready"})
    };
    (status, Json(body)).into_response()
}

async fn readiness_status(
    postgres: &dyn ReadinessProbe,
    source_storage: Option<&dyn ReadinessProbe>,
) -> StatusCode {
    let (postgres_ready, storage_ready) = tokio::join!(postgres.is_ready(), async {
        match source_storage {
            Some(probe) => probe.is_ready().await,
            None => true,
        }
    });
    if postgres_ready && storage_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

#[cfg(test)]
mod ownership_tests {
    use super::*;
    use async_trait::async_trait;

    struct FixedProbe(bool);

    #[async_trait]
    impl ReadinessProbe for FixedProbe {
        async fn is_ready(&self) -> bool {
            self.0
        }
    }

    #[test]
    fn public_novel_json_omits_uploader_identity() {
        let novel = Novel::create(Uuid::new_v4(), "Shared novel".into(), None);
        let json = serde_json::to_value(novel).unwrap();

        assert!(json.get("user_id").is_none());
    }

    #[test]
    fn routes_construct_with_axum_08_syntax() {
        let _ = routes();
    }

    #[test]
    fn import_metadata_is_bounded_and_mode_is_strict() {
        let (title, author, mode) = validate_metadata(
            "  Story  ".into(),
            Some(" Author ".into()),
            Some("creative".into()),
        )
        .unwrap();
        assert_eq!(title, "Story");
        assert_eq!(author.as_deref(), Some("Author"));
        assert!(matches!(
            mode,
            Some(crate::domain::value_objects::DeviationMode::Creative)
        ));
        assert_eq!(
            validate_metadata("x".repeat(MAX_TITLE_CHARS + 1), None, None)
                .unwrap_err()
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
        assert_eq!(
            validate_metadata("Story".into(), None, Some("surprise".into()))
                .unwrap_err()
                .0,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn readiness_status_fails_closed() {
        assert_eq!(
            readiness_status(&FixedProbe(true), None).await,
            StatusCode::OK
        );
        assert_eq!(
            readiness_status(&FixedProbe(false), None).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(
            readiness_status(&FixedProbe(true), Some(&FixedProbe(false))).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    #[tokio::test]
    async fn retry_errors_expose_only_safe_messages() {
        let internal = retry_error_response(anyhow::anyhow!("database-secret"), Uuid::nil());
        assert_eq!(internal.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(internal.into_body(), 1_024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"error": "Internal server error"})
        );

        let conflict = retry_error_response(
            ImportRetryConflict("Import cannot be retried").into(),
            Uuid::nil(),
        );
        assert_eq!(conflict.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(conflict.into_body(), 1_024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({"error": "Import cannot be retried"})
        );
    }

    #[tokio::test]
    async fn coded_game_rule_errors_are_machine_readable() {
        let response = coded_api_error(
            StatusCode::UNPROCESSABLE_ENTITY,
            "game_rule_generation_exhausted",
            "Game rule generation budget is exhausted",
        );
        let body = axum::body::to_bytes(response.into_body(), 1_024)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({
                "error": {
                    "code": "game_rule_generation_exhausted",
                    "message": "Game rule generation budget is exhausted"
                }
            }),
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
}
