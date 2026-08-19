use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Json, Path, Query, State},
    http::{
        header::{CACHE_CONTROL, RETRY_AFTER},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{sse::Event, sse::KeepAlive, IntoResponse, Response, Sse},
    routing::{get, post},
    Router,
};
use futures::StreamExt;
use serde::{de::IgnoredAny, Deserialize, Deserializer, Serialize};
use std::sync::Arc;
use uuid::{Uuid, Variant, Version};

use crate::application::handlers::{AgentCommandHandler, AgentRequestError, AgentStreamEvent};
use crate::domain::entities::memory::MemoryLayer;
use crate::domain::ports::{AccountExportPort, ReadinessProbe};
use crate::domain::services::memory_manager::PermanentMemorySave;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<AgentCommandHandler>,
    pub postgres_readiness: Arc<dyn ReadinessProbe>,
    pub redis_readiness: Arc<dyn ReadinessProbe>,
    pub novel_readiness: Arc<dyn ReadinessProbe>,
    pub narrative_readiness: Arc<dyn ReadinessProbe>,
    pub account_export: Arc<dyn AccountExportPort>,
    pub internal_service_token: Arc<str>,
    pub metrics: llm_client::MetricsHandle,
}

pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

fn routes() -> Router<AppState> {
    Router::new()
        // 流式对话（SSE）
        .route("/chat/{character_id}/stream", post(chat_stream))
        // 普通对话
        .route("/chat/{character_id}", post(chat))
        // 获取对话历史
        .route("/chat/{character_id}/history", get(get_history))
        // 获取角色记忆
        .route("/memories/{character_id}", get(get_memories))
        // 清除短期记忆
        .route(
            "/memories/{character_id}/short",
            axum::routing::delete(clear_short_memory),
        )
        .route(
            "/internal/privacy/users/{user_id}",
            axum::routing::delete(clear_user_cache),
        )
        .route(
            "/internal/privacy/users/{user_id}/novels/{novel_id}",
            axum::routing::delete(clear_novel_cache),
        )
        .route(
            "/internal/privacy/tombstones/users/{user_id}",
            axum::routing::delete(allow_user_cache),
        )
        .route(
            "/internal/privacy/tombstones/users/{user_id}/novels/{novel_id}",
            axum::routing::delete(allow_novel_cache),
        )
        .route(
            "/internal/privacy/users/{user_id}/export",
            get(export_account),
        )
        .route("/internal/memories", post(save_permanent_memory))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .layer(DefaultBodyLimit::max(MAX_CHAT_BODY_BYTES))
}

async fn export_account(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let stream = state.account_export.export_user(user_id).map(|result| {
        let record = result.map_err(std::io::Error::other)?;
        let mut line = serde_json::to_vec(&serde_json::json!({
            "type": "record",
            "service": "agent",
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

/// Bound for world-journey event text recorded as a permanent memory.
const MAX_PERMANENT_EVENT_CHARS: usize = 2_000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SavePermanentMemoryRequest {
    /// Caller-supplied idempotency key (narrative's committed turn id); the
    /// repository upsert makes a replay idempotent.
    memory_id: Uuid,
    character_id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    chapter_number: i32,
    event: String,
    importance: i32,
}

/// Validation for a permanent-memory write; separate from the handler so
/// the contract is unit-testable without a full router stack.
fn validate_permanent_memory_request(
    event: &str,
    chapter_number: i32,
    importance: i32,
) -> Result<(), &'static str> {
    let event = event.trim();
    if event.is_empty() || event.chars().count() > MAX_PERMANENT_EVENT_CHARS {
        return Err("event must be 1..=2000 chars");
    }
    if chapter_number < 1 {
        return Err("chapter_number must be >= 1");
    }
    if !(1..=10).contains(&importance) {
        return Err("importance must be 1..=10");
    }
    Ok(())
}

/// POST /internal/memories — internal-token-protected producer for permanent
/// memories, called by narrative-service after a committed world turn.
async fn save_permanent_memory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<SavePermanentMemoryRequest>,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let event = request.event.trim();
    if let Err(message) =
        validate_permanent_memory_request(event, request.chapter_number, request.importance)
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({"error": message})),
        )
            .into_response();
    }
    match state
        .handler
        .memory_manager
        .save_permanent_memory(
            request.memory_id,
            request.character_id,
            request.user_id,
            request.novel_id,
            request.chapter_number,
            event,
            request.importance,
        )
        .await
    {
        Ok(PermanentMemorySave::Saved) => {
            (StatusCode::OK, Json(serde_json::json!({"saved": true}))).into_response()
        }
        Ok(PermanentMemorySave::SkippedWrongDimensions) => {
            // Non-retryable policy skip (SPEC 6.2.4): honest 200 so the caller
            // does not burn its retry budget on a config issue.
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "saved": false,
                    "reason": "embedding dimension policy"
                })),
            )
                .into_response()
        }
        Ok(PermanentMemorySave::SkippedEmbeddingUnavailable) => {
            // Retryable: surface as an error so the caller's bounded retry
            // loop covers transient embedding-provider outages.
            tracing::warn!("permanent memory skipped: embedding unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "saved": false,
                    "reason": "embedding unavailable"
                })),
            )
                .into_response()
        }
        Err(error) => {
            tracing::error!(%error, "permanent memory save failed");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
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

const MAX_CHAT_BODY_BYTES: usize = 20 * 1024;
const MAX_CHAT_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_CHAT_MESSAGE_CHARS: usize = 4_000;
const MAX_HISTORY_LIMIT: i64 = 100;
const MAX_HISTORY_OFFSET: i64 = 10_000;

fn provided<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    IgnoredAny::deserialize(deserializer)?;
    Ok(true)
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChatRequest {
    pub message: String,
    /// Old clients supplied a body principal. It is never trusted and acts as
    /// a stable upgrade marker so stale tabs cannot chat with stale context.
    #[serde(default, rename = "user_id", deserialize_with = "provided")]
    pub _legacy_user_id: bool,
    #[serde(default)]
    pub novel_id: Option<Uuid>,
    #[serde(default)]
    pub reader_identity: Option<String>,
    #[serde(default)]
    pub current_chapter: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub message: String,
    pub character_id: Uuid,
    pub turn_id: Uuid,
    pub committed: bool,
    pub replayed: bool,
}

fn validation_error(message: impl Into<String>) -> axum::response::Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(serde_json::json!({
            "error": {"code": "validation_error", "message": message.into()}
        })),
    )
        .into_response()
}

fn client_upgrade_required(user_id: Uuid) -> axum::response::Response {
    tracing::warn!(%user_id, "legacy chat client rejected; refresh required");
    (
        StatusCode::UPGRADE_REQUIRED,
        Json(serde_json::json!({
            "error": {
                "code": "client_upgrade_required",
                "message": "Refresh the application before starting a new chat"
            }
        })),
    )
        .into_response()
}

fn application_error_response(
    error: anyhow::Error,
    fallback_status: StatusCode,
) -> axum::response::Response {
    let retry_after = match error.downcast_ref::<AgentRequestError>() {
        Some(AgentRequestError::TurnInProgress {
            retry_after_seconds,
        }) => Some(*retry_after_seconds),
        Some(AgentRequestError::Capacity {
            retry_after_seconds,
        }) => Some(*retry_after_seconds),
        _ => None,
    };
    let (status, code, message) = match error.downcast_ref::<AgentRequestError>() {
        Some(AgentRequestError::NotFound) => (
            StatusCode::NOT_FOUND,
            "not_found",
            "Character not found".to_string(),
        ),
        Some(AgentRequestError::Validation(message)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            message.clone(),
        ),
        Some(AgentRequestError::TurnInProgress { .. }) => (
            StatusCode::CONFLICT,
            "turn_in_progress",
            "Chat turn is already in progress".to_string(),
        ),
        Some(AgentRequestError::Capacity { .. }) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "capacity_unavailable",
            "Chat capacity is busy; retry the request".to_string(),
        ),
        Some(AgentRequestError::TurnConflict) => (
            StatusCode::CONFLICT,
            "idempotency_conflict",
            "Idempotency key conflicts with an existing chat turn".to_string(),
        ),
        Some(AgentRequestError::Llm(_)) => (
            StatusCode::BAD_GATEWAY,
            "llm_error",
            "The language model could not complete the request".to_string(),
        ),
        Some(AgentRequestError::Unavailable(_)) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "dependency_unavailable",
            "Required service is unavailable".to_string(),
        ),
        None => (
            fallback_status,
            "internal_error",
            "Request could not be completed".to_string(),
        ),
    };
    if status.is_server_error() {
        tracing::error!(error = ?error, "agent request failed");
    }
    let mut response = (
        status,
        Json(serde_json::json!({
            "error": {"code": code, "message": message}
        })),
    )
        .into_response();
    if let Some(seconds) = retry_after {
        if let Ok(value) = HeaderValue::from_str(&seconds.to_string()) {
            response.headers_mut().insert(RETRY_AFTER, value);
        }
    }
    response
}

fn validate_message(message: String) -> Result<String, String> {
    let message = message.trim().to_string();
    if message.is_empty() {
        return Err("message must not be empty".into());
    }
    if message.len() > MAX_CHAT_MESSAGE_BYTES || message.chars().count() > MAX_CHAT_MESSAGE_CHARS {
        return Err(format!(
            "message must not exceed {MAX_CHAT_MESSAGE_CHARS} characters or {MAX_CHAT_MESSAGE_BYTES} bytes"
        ));
    }
    Ok(message)
}

fn extract_turn_id(headers: &HeaderMap) -> Result<Option<Uuid>, String> {
    let Some(value) = headers.get("idempotency-key") else {
        return Ok(None);
    };
    let value = value
        .to_str()
        .map_err(|_| "idempotency key must be a UUID v4".to_string())?;
    let turn_id =
        Uuid::parse_str(value).map_err(|_| "idempotency key must be a UUID v4".to_string())?;
    if turn_id.get_version() != Some(Version::Random) || turn_id.get_variant() != Variant::RFC4122 {
        return Err("idempotency key must be a UUID v4".into());
    }
    Ok(Some(turn_id))
}

/// 流式 SSE 对话接口
async fn chat_stream(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> axum::response::Response {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Missing or invalid X-User-Id header"})),
            )
                .into_response();
        }
    };
    if req._legacy_user_id {
        return client_upgrade_required(user_id);
    }
    let turn_id = match extract_turn_id(&headers) {
        Ok(Some(turn_id)) => turn_id,
        Ok(None) => return client_upgrade_required(user_id),
        Err(message) => return validation_error(message),
    };
    let message = match validate_message(req.message) {
        Ok(message) => message,
        Err(message) => return validation_error(message),
    };

    let handler = state.handler.clone();
    let source = match handler
        .chat_stream(turn_id, character_id, user_id, req.novel_id, message)
        .await
    {
        Ok(source) => source,
        Err(error) => {
            return application_error_response(error, StatusCode::SERVICE_UNAVAILABLE);
        }
    };

    let stream = async_stream::stream! {
        let _handler = handler;
        let mut source = std::pin::pin!(source);
        let mut terminal = false;
        while let Some(item) = source.next().await {
            match item {
                Ok(AgentStreamEvent::Delta(text)) if !text.is_empty() => {
                    yield Ok::<Event, anyhow::Error>(
                        Event::default()
                            .event("delta")
                            .data(serde_json::json!({"content": text}).to_string())
                    );
                }
                Ok(AgentStreamEvent::Done { replayed }) => {
                    terminal = true;
                    yield Ok(Event::default()
                        .event("done")
                        .data(serde_json::json!({
                            "turn_id": turn_id,
                            "committed": true,
                            "replayed": replayed
                        }).to_string()));
                    break;
                }
                Err(_) => {
                    yield Ok(Event::default()
                        .event("error")
                        .data(serde_json::json!({
                            "code": "turn_failed",
                            "message": "Response could not be completed; retry is safe",
                            "turn_id": turn_id
                        }).to_string()));
                    return;
                }
                _ => {}
            }
        }
        if !terminal {
            yield Ok(Event::default()
                .event("error")
                .data(serde_json::json!({
                    "code": "stream_ended_early",
                    "message": "Response ended before commit confirmation",
                    "turn_id": turn_id
                }).to_string()));
        }
    };

    Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// 普通对话接口（非流式）
async fn chat(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({"error": "Missing or invalid X-User-Id header"})),
            )
                .into_response();
        }
    };
    if req._legacy_user_id {
        return client_upgrade_required(user_id);
    }
    let turn_id = match extract_turn_id(&headers) {
        Ok(Some(turn_id)) => turn_id,
        Ok(None) => return client_upgrade_required(user_id),
        Err(message) => return validation_error(message),
    };
    let message = match validate_message(req.message) {
        Ok(message) => message,
        Err(message) => return validation_error(message),
    };

    match state
        .handler
        .chat(turn_id, character_id, user_id, req.novel_id, message)
        .await
    {
        Ok(response) => (
            StatusCode::OK,
            Json(ChatResponse {
                message: response.message,
                character_id,
                turn_id,
                committed: true,
                replayed: response.replayed,
            }),
        )
            .into_response(),
        Err(error) => application_error_response(error, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Query params for chat history pagination.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryQuery {
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

/// GET /chat/:character_id/history?limit=50&offset=0
async fn get_history(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
    headers: HeaderMap,
    Query(params): Query<HistoryQuery>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Missing or invalid X-User-Id header"
                })),
            )
                .into_response()
        }
    };
    if !(1..=MAX_HISTORY_LIMIT).contains(&params.limit)
        || !(0..=MAX_HISTORY_OFFSET).contains(&params.offset)
    {
        return validation_error(format!(
            "limit must be between 1 and {MAX_HISTORY_LIMIT}; offset must be between 0 and {MAX_HISTORY_OFFSET}"
        ));
    }
    match state
        .handler
        .get_history(character_id, user_id, params.limit, params.offset)
        .await
    {
        Ok(messages) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "messages": messages,
                "count": messages.len(),
            })),
        )
            .into_response(),
        Err(error) => application_error_response(error, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Query params for memory retrieval.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryQuery {
    novel_id: Uuid,
    /// One of: "short", "mid", "long", "permanent". Defaults to "permanent".
    #[serde(default = "default_layer")]
    layer: String,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_layer() -> String {
    "permanent".into()
}

fn parse_layer(s: &str) -> Option<MemoryLayer> {
    match s {
        "short" => Some(MemoryLayer::Short),
        "mid" => Some(MemoryLayer::Mid),
        "long" => Some(MemoryLayer::Long),
        "permanent" => Some(MemoryLayer::Permanent),
        _ => None,
    }
}

/// GET /memories/:character_id?novel_id=...&layer=permanent
async fn get_memories(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
    headers: HeaderMap,
    Query(params): Query<MemoryQuery>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Missing or invalid X-User-Id header"
                })),
            )
                .into_response()
        }
    };
    let layer = match parse_layer(&params.layer) {
        Some(layer) => layer,
        None => return validation_error("layer must be short, mid, long, or permanent"),
    };
    if !(1..=MAX_HISTORY_LIMIT).contains(&params.limit)
        || !(0..=MAX_HISTORY_OFFSET).contains(&params.offset)
    {
        return validation_error(format!(
            "limit must be between 1 and {MAX_HISTORY_LIMIT}; offset must be between 0 and {MAX_HISTORY_OFFSET}"
        ));
    }
    match state
        .handler
        .get_memories(
            character_id,
            user_id,
            params.novel_id,
            layer,
            params.limit,
            params.offset,
        )
        .await
    {
        Ok(memories) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "memories": memories,
                "count": memories.len(),
            })),
        )
            .into_response(),
        Err(error) => application_error_response(error, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

/// Query params for clearing short-term memory.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClearShortQuery {}

/// DELETE /memories/:character_id/short
async fn clear_short_memory(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
    headers: HeaderMap,
    Query(_params): Query<ClearShortQuery>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Missing or invalid X-User-Id header"
                })),
            )
                .into_response()
        }
    };
    match state
        .handler
        .clear_short_memory(character_id, user_id)
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => application_error_response(error, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn clear_user_cache(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !internal_request_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.handler.clear_user_cache(user_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => application_error_response(error, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn clear_novel_cache(
    State(state): State<AppState>,
    Path((user_id, novel_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !internal_request_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.handler.clear_novel_cache(user_id, novel_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => application_error_response(error, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn allow_user_cache(
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !internal_request_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.handler.allow_user_cache(user_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => application_error_response(error, StatusCode::INTERNAL_SERVER_ERROR),
    }
}

async fn allow_novel_cache(
    State(state): State<AppState>,
    Path((user_id, novel_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if !internal_request_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    match state.handler.allow_novel_cache(user_id, novel_id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => application_error_response(error, StatusCode::INTERNAL_SERVER_ERROR),
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

/// Extract user_id from X-User-Id header; returns None if missing or invalid.
fn extract_user_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK").into_response()
}

async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let (postgres_ready, redis_ready, novel_ready, narrative_ready) = dependency_readiness(
        state.postgres_readiness.as_ref(),
        state.redis_readiness.as_ref(),
        state.novel_readiness.as_ref(),
        state.narrative_readiness.as_ref(),
    )
    .await;
    let status = if postgres_ready && redis_ready && novel_ready && narrative_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (
        status,
        Json(serde_json::json!({
            "status": if status == StatusCode::OK { "ready" } else { "not_ready" },
            "dependencies": {
                "postgres": postgres_ready,
                "redis": redis_ready,
                "novel_service": novel_ready,
                "narrative_service": narrative_ready,
            }
        })),
    )
        .into_response()
}

async fn dependency_readiness(
    postgres: &dyn ReadinessProbe,
    redis: &dyn ReadinessProbe,
    novel: &dyn ReadinessProbe,
    narrative: &dyn ReadinessProbe,
) -> (bool, bool, bool, bool) {
    tokio::join!(
        postgres.is_ready(),
        redis.is_ready(),
        novel.is_ready(),
        narrative.is_ready()
    )
}

#[cfg(test)]
mod principal_contract_tests {
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
    fn body_principal_marks_a_client_that_must_upgrade() {
        let forged = "demo-user";
        let request = serde_json::from_value::<ChatRequest>(serde_json::json!({
            "user_id": forged,
            "novel_id": Uuid::new_v4(),
            "message": "hello",
            "current_chapter": 1
        }))
        .unwrap();
        assert!(request._legacy_user_id);
        let null_marker = serde_json::from_value::<ChatRequest>(serde_json::json!({
            "user_id": null,
            "message": "hello"
        }))
        .unwrap();
        assert!(null_marker._legacy_user_id);
        assert_eq!(
            client_upgrade_required(Uuid::new_v4()).status(),
            StatusCode::UPGRADE_REQUIRED
        );
        assert!(serde_json::from_value::<HistoryQuery>(serde_json::json!({
            "user_id": Uuid::new_v4()
        }))
        .is_err());
        assert!(serde_json::from_value::<MemoryQuery>(serde_json::json!({
            "user_id": Uuid::new_v4(),
            "novel_id": Uuid::new_v4()
        }))
        .is_err());
        assert!(
            serde_json::from_value::<ClearShortQuery>(serde_json::json!({
                "user_id": Uuid::new_v4()
            }))
            .is_err()
        );
    }

    #[test]
    fn chat_request_accepts_canonical_and_legacy_payloads() {
        let canonical = serde_json::from_value::<ChatRequest>(serde_json::json!({
            "message": "hello"
        }))
        .unwrap();
        assert_eq!(canonical.message, "hello");
        assert!(!canonical._legacy_user_id);
        assert!(canonical.novel_id.is_none());

        let legacy = serde_json::from_value::<ChatRequest>(serde_json::json!({
            "message": "hello",
            "novel_id": Uuid::new_v4(),
            "reader_identity": "forged",
            "current_chapter": 999
        }))
        .unwrap();
        assert_eq!(legacy.reader_identity.as_deref(), Some("forged"));
        assert_eq!(legacy.current_chapter, Some(999));
    }

    #[test]
    fn message_validation_enforces_trimmed_character_and_byte_limits() {
        assert!(validate_message(" \n\t ".into()).is_err());
        assert_eq!(validate_message(" hello ".into()).unwrap(), "hello");
        assert!(validate_message("a".repeat(MAX_CHAT_MESSAGE_CHARS)).is_ok());
        assert!(validate_message("a".repeat(MAX_CHAT_MESSAGE_CHARS + 1)).is_err());
        assert!(validate_message("读".repeat(MAX_CHAT_MESSAGE_CHARS)).is_ok());
        assert!(validate_message("🦀".repeat(MAX_CHAT_MESSAGE_CHARS)).is_ok());
        assert!(validate_message("🦀".repeat(MAX_CHAT_MESSAGE_CHARS + 1)).is_err());
    }

    #[test]
    fn idempotency_key_requires_uuid_v4_and_distinguishes_old_clients() {
        let mut headers = HeaderMap::new();
        assert_eq!(extract_turn_id(&headers).unwrap(), None);
        headers.insert("idempotency-key", HeaderValue::from_static("not-a-uuid"));
        assert!(extract_turn_id(&headers).is_err());
        headers.insert(
            "idempotency-key",
            HeaderValue::from_str(&Uuid::nil().to_string()).unwrap(),
        );
        assert!(extract_turn_id(&headers).is_err());
        headers.insert(
            "idempotency-key",
            HeaderValue::from_static("00000000-0000-4000-0000-000000000001"),
        );
        assert!(extract_turn_id(&headers).is_err());
        let turn_id = Uuid::new_v4();
        headers.insert(
            "idempotency-key",
            HeaderValue::from_str(&turn_id.to_string()).unwrap(),
        );
        assert_eq!(extract_turn_id(&headers).unwrap(), Some(turn_id));
    }

    #[test]
    fn in_progress_response_exposes_the_remaining_lease() {
        let response = application_error_response(
            AgentRequestError::TurnInProgress {
                retry_after_seconds: 117,
            }
            .into(),
            StatusCode::SERVICE_UNAVAILABLE,
        );
        assert_eq!(response.status(), StatusCode::CONFLICT);
        assert_eq!(response.headers().get(RETRY_AFTER).unwrap(), "117");
    }

    #[test]
    fn permanent_memory_request_validation_is_strict() {
        assert_eq!(
            validate_permanent_memory_request("", 1, 5),
            Err("event must be 1..=2000 chars")
        );
        assert_eq!(
            validate_permanent_memory_request(
                "x".repeat(MAX_PERMANENT_EVENT_CHARS + 1).as_str(),
                1,
                5
            ),
            Err("event must be 1..=2000 chars")
        );
        assert_eq!(
            validate_permanent_memory_request("ok", 0, 5),
            Err("chapter_number must be >= 1")
        );
        assert_eq!(
            validate_permanent_memory_request("ok", 1, 0),
            Err("importance must be 1..=10")
        );
        assert_eq!(
            validate_permanent_memory_request("ok", 1, 11),
            Err("importance must be 1..=10")
        );
        assert!(validate_permanent_memory_request("ok", 1, 5).is_ok());
    }

    #[test]
    fn memory_layer_is_strict() {
        assert_eq!(parse_layer("short"), Some(MemoryLayer::Short));
        assert_eq!(parse_layer("permanent"), Some(MemoryLayer::Permanent));
        assert_eq!(parse_layer("unknown"), None);
    }

    #[test]
    fn routes_construct_with_axum_08_syntax() {
        let _ = routes();
    }

    #[test]
    fn internal_export_auth_rejects_missing_and_wrong_tokens() {
        let mut headers = HeaderMap::new();
        assert!(!internal_token_authorized(&headers, "expected-token"));
        headers.insert(
            "X-Internal-Service-Token",
            HeaderValue::from_static("wrong-token"),
        );
        assert!(!internal_token_authorized(&headers, "expected-token"));
        headers.insert(
            "X-Internal-Service-Token",
            HeaderValue::from_static("expected-token"),
        );
        assert!(internal_token_authorized(&headers, "expected-token"));
    }

    #[tokio::test]
    async fn dependency_readiness_fails_closed_per_dependency() {
        assert_eq!(
            dependency_readiness(
                &FixedProbe(true),
                &FixedProbe(false),
                &FixedProbe(true),
                &FixedProbe(false),
            )
            .await,
            (true, false, true, false)
        );
    }
}
