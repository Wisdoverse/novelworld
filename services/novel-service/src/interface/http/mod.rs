use axum::{
    extract::{DefaultBodyLimit, Json, Multipart, Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::application::commands::ImportNovelCommand;
use crate::application::handlers::{
    NovelCommandHandler, ReadingProgressError, ReadingProgressHandler,
};
use crate::domain::entities::novel::Novel;
use crate::domain::ports::ReadinessProbe;
use crate::domain::repositories::{ChapterRepository, CharacterRepository, NovelRepository};
use axum::routing::put;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<NovelCommandHandler>,
    pub novel_repo: Arc<dyn NovelRepository>,
    pub chapter_repo: Arc<dyn ChapterRepository>,
    pub character_repo: Arc<dyn CharacterRepository>,
    pub progress_handler: Arc<ReadingProgressHandler>,
    pub readiness: Arc<dyn ReadinessProbe>,
}

pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

fn routes() -> Router<AppState> {
    Router::new()
        .route("/novels", post(import_novel))
        .route("/novels/upload", post(upload_novel))
        .route("/novels", get(list_novels))
        .route("/novels/{id}", get(get_novel))
        .route("/novels/{id}", delete(delete_novel))
        .route("/novels/{id}/chapters", get(list_chapters))
        .route("/novels/{id}/chapters/{num}", get(get_chapter))
        .route("/novels/{id}/lore/search", post(search_lore))
        .route("/novels/{id}/characters", get(list_characters))
        .route("/characters/{id}", get(get_character_by_id))
        .route("/novels/{id}/relationships", get(list_relationships))
        .route("/novels/{id}/status", get(get_parse_status))
        .route("/progress/{novel_id}", get(get_progress))
        .route("/progress/{novel_id}", put(update_progress))
        .route("/progress/{novel_id}/identity", put(set_identity))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .layer(DefaultBodyLimit::max(MAX_REQUEST_SIZE))
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
        file_key: None,
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

/// Maximum upload size for plain text files (10 MB).
const MAX_TXT_SIZE: usize = 10 * 1024 * 1024;
/// Maximum upload size for PDF files (20 MB).
const MAX_PDF_SIZE: usize = 20 * 1024 * 1024;
const MAX_PASTE_SIZE: usize = 5 * 1024 * 1024;
const MAX_REQUEST_SIZE: usize = MAX_PDF_SIZE + 1024 * 1024;
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

/// Extract text content from an in-memory PDF using pdf-extract.
fn extract_pdf_text(data: &[u8]) -> Result<String, (StatusCode, axum::Json<ApiError>)> {
    pdf_extract::extract_text_from_mem(data).map_err(|e| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(ApiError {
                error: format!("Failed to extract text from PDF: {}", e),
            }),
        )
    })
}

/// POST /novels/upload  -- multipart file upload (txt or pdf)
async fn upload_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
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

    let mut title: Option<String> = None;
    let mut author: Option<String> = None;
    let mut content: Option<String> = None;
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
                let content_type = field
                    .content_type()
                    .unwrap_or("application/octet-stream")
                    .to_string();
                let data = match field.bytes().await {
                    Ok(d) => d,
                    Err(e) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(ApiError {
                                error: format!("Failed to read uploaded file: {}", e),
                            }),
                        )
                            .into_response();
                    }
                };

                let is_pdf = content_type.contains("pdf");

                // Validate size limits
                let max_size = if is_pdf { MAX_PDF_SIZE } else { MAX_TXT_SIZE };
                if data.len() > max_size {
                    return (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        Json(ApiError {
                            error: format!(
                                "File too large: {} bytes (max {} bytes for {})",
                                data.len(),
                                max_size,
                                if is_pdf { "PDF" } else { "text" }
                            ),
                        }),
                    )
                        .into_response();
                }

                // Validate MIME type
                if !is_pdf
                    && !content_type.contains("text/plain")
                    && !content_type.contains("octet-stream")
                {
                    return (
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        Json(ApiError {
                            error: format!(
                                "Unsupported file type: {}. Only text/plain and application/pdf are accepted.",
                                content_type
                            ),
                        }),
                    )
                        .into_response();
                }

                // Extract text
                if is_pdf {
                    match extract_pdf_text(&data) {
                        Ok(text) if text.len() <= MAX_PDF_SIZE => content = Some(text),
                        Ok(_) => {
                            return api_error(
                                StatusCode::PAYLOAD_TOO_LARGE,
                                "Extracted PDF text exceeds 20 MiB",
                            )
                        }
                        Err(resp) => return resp.into_response(),
                    }
                } else {
                    content = Some(String::from_utf8_lossy(&data).to_string());
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
        file_key: None,
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
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
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

async fn get_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    match owned_novel(&state, &headers, id).await {
        Ok(novel) => (StatusCode::OK, Json(novel)).into_response(),
        Err(response) => response,
    }
}

async fn delete_novel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(response) = owned_novel(&state, &headers, id).await {
        return response;
    }
    match state.novel_repo.delete(id).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response(),
    }
}

async fn list_chapters(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(novel_id): Path<Uuid>,
) -> impl IntoResponse {
    if let Err(response) = owned_novel(&state, &headers, novel_id).await {
        return response;
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
        return response;
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
        return response;
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
                "status": format!("{:?}", novel.status),
                "total_chapters": novel.total_chapters,
                "error": novel.parse_error,
            })),
        )
            .into_response(),
        Err(response) => response,
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

async fn owned_novel(
    state: &AppState,
    headers: &HeaderMap,
    novel_id: Uuid,
) -> Result<Novel, Response> {
    let user_id = extract_user_id(headers).ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            Json(ApiError {
                error: "Missing or invalid X-User-Id header".into(),
            }),
        )
            .into_response()
    })?;

    match state.novel_repo.find_by_id(novel_id).await {
        Ok(Some(novel)) if is_novel_owner(&novel, user_id) => Ok(novel),
        Ok(_) => Err((
            StatusCode::NOT_FOUND,
            Json(ApiError {
                error: "Novel not found".into(),
            }),
        )
            .into_response()),
        Err(e) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ApiError {
                error: e.to_string(),
            }),
        )
            .into_response()),
    }
}

fn is_novel_owner(novel: &Novel, user_id: Uuid) -> bool {
    novel.user_id == user_id
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
    let status = readiness_status(state.readiness.as_ref()).await;
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

async fn readiness_status(probe: &dyn ReadinessProbe) -> StatusCode {
    if probe.is_ready().await {
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
    fn novel_ownership_is_principal_scoped() {
        let owner_id = Uuid::new_v4();
        let novel = Novel::create(owner_id, "Private novel".into(), None);

        assert!(is_novel_owner(&novel, owner_id));
        assert!(!is_novel_owner(&novel, Uuid::new_v4()));
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
        assert_eq!(readiness_status(&FixedProbe(true)).await, StatusCode::OK);
        assert_eq!(
            readiness_status(&FixedProbe(false)).await,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }
}
