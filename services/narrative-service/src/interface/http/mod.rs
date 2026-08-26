use axum::{
    body::Body,
    extract::{DefaultBodyLimit, Json, Path, Query, State},
    http::{
        header::{CACHE_CONTROL, RETRY_AFTER},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::{Uuid, Variant, Version};

use crate::application::handlers::{
    CreatePlayerEntityCommand, NarrativeCommandHandler, NarrativeError,
};
use crate::domain::entities::game_rules::PlayerRuleProfile;
use crate::domain::entities::world_session::{
    CharacterContextEnvelope, CharacterWorldContext, WorldAction, WorldActionKind,
};
use crate::domain::ports::{AccountExportPort, ReadinessProbe};

const WORLD_CONTEXT_VERSION_HEADER: &str = "X-World-Context-Version";
const WORLD_CONTEXT_VERSION_V2: &str = "2";
const WORLD_CONTEXT_VERSION_V3: &str = "3";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorldContextVersion {
    V2,
    V3,
}

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<NarrativeCommandHandler>,
    pub postgres_readiness: Arc<dyn ReadinessProbe>,
    pub novel_readiness: Arc<dyn ReadinessProbe>,
    pub account_export: Arc<dyn AccountExportPort>,
    pub internal_service_token: Arc<str>,
    pub metrics: llm_client::MetricsHandle,
}

pub fn router(state: AppState) -> Router {
    routes().with_state(state)
}

fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/narrative/{novel_id}/chapters/{chapter}",
            get(get_effective_chapter),
        )
        .route(
            "/narrative/{novel_id}/player-entry",
            get(get_player_entry)
                .put(put_player_entry)
                .layer(DefaultBodyLimit::max(64 * 1024)),
        )
        .route(
            "/narrative/{novel_id}/game-rules",
            post(request_game_rule_template),
        )
        .route("/narrative/{novel_id}/{chapter}", get(get_branch_node))
        .route("/narrative/choose", post(submit_choice))
        .route("/narrative/{novel_id}/world-state", get(get_world_state))
        .route(
            "/narrative/{novel_id}/world",
            get(get_open_world).post(start_open_world),
        )
        .route("/narrative/{novel_id}/world/turns", post(submit_world_turn))
        .route(
            "/internal/privacy/users/{user_id}/export",
            get(export_account),
        )
        .route(
            "/internal/narrative/{novel_id}/characters/{character_id}/context",
            get(get_character_world_context),
        )
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/metrics", get(metrics))
        .layer(DefaultBodyLimit::max(4 * 1024))
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
            "service": "narrative",
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

async fn get_character_world_context(
    State(state): State<AppState>,
    Path((novel_id, character_id)): Path<(Uuid, Uuid)>,
    headers: HeaderMap,
) -> Response {
    if !internal_request_authorized(&state, &headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let Some(user_id) = extract_user_id(&headers) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    match world_context_version(&headers) {
        Some(WorldContextVersion::V2) => character_world_context_response(
            state
                .handler
                .get_character_world_context(user_id, novel_id, character_id)
                .await,
        ),
        Some(WorldContextVersion::V3) => character_context_envelope_response(
            state
                .handler
                .get_character_context_envelope(user_id, novel_id, character_id)
                .await,
        ),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

fn character_world_context_response(
    result: Result<Option<CharacterWorldContext>, NarrativeError>,
) -> Response {
    match result {
        Ok(Some(context)) => (StatusCode::OK, Json(context)).into_response(),
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => narrative_error_response(error),
    }
}

fn character_context_envelope_response(
    result: Result<Option<CharacterContextEnvelope>, NarrativeError>,
) -> Response {
    match result {
        Ok(Some(context)) => (StatusCode::OK, Json(context)).into_response(),
        Ok(None) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => narrative_error_response(error),
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

// ─── Request/Response DTOs ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SubmitChoiceRequest {
    pub novel_id: Uuid,
    pub node_id: Uuid,
    pub choice_index: i32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePlayerEntityRequest {
    pub checkpoint_chapter: Option<i32>,
    pub name: String,
    pub background: String,
    pub capabilities: Vec<String>,
    pub location_id: String,
    pub inventory: Vec<String>,
    #[serde(default)]
    pub rules: PlayerRuleProfile,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayerEntryQuery {
    pub checkpoint_chapter: Option<i32>,
}

#[derive(Debug, Serialize)]
pub struct ApiError {
    pub error: ErrorDetail,
}

#[derive(Debug, Serialize)]
pub struct ErrorDetail {
    pub code: &'static str,
    pub message: String,
}

fn error_response(
    status: StatusCode,
    code: &'static str,
    message: &str,
) -> axum::response::Response {
    (
        status,
        Json(ApiError {
            error: ErrorDetail {
                code,
                message: message.into(),
            },
        }),
    )
        .into_response()
}

fn narrative_error_response(error: NarrativeError) -> axum::response::Response {
    let (status, code, message) = match error {
        NarrativeError::NotFound => (StatusCode::NOT_FOUND, "not_found", "Resource not found"),
        NarrativeError::Validation(message) => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "validation_error",
                &message,
            )
        }
        NarrativeError::Conflict(message) => {
            return error_response(StatusCode::CONFLICT, "conflict", &message)
        }
        NarrativeError::ChoiceConflict => {
            return error_response(
                StatusCode::CONFLICT,
                "choice_conflict",
                "A different choice is already committed",
            )
        }
        NarrativeError::TurnInProgress {
            retry_after_seconds,
        } => {
            let mut response = error_response(
                StatusCode::CONFLICT,
                "turn_in_progress",
                "World turn is already in progress",
            );
            if let Ok(value) = HeaderValue::from_str(&retry_after_seconds.to_string()) {
                response.headers_mut().insert(RETRY_AFTER, value);
            }
            return response;
        }
        NarrativeError::TurnOutcomeUnknown => {
            return error_response(
                StatusCode::CONFLICT,
                "turn_outcome_unknown",
                "World turn outcome is unknown; retry with the same Idempotency-Key",
            )
        }
        NarrativeError::GameRulesInProgress {
            retry_after_seconds,
        } => {
            let mut response = error_response(
                StatusCode::CONFLICT,
                "game_rule_generation_in_progress",
                "Game rule template generation is in progress",
            );
            if let Ok(value) = HeaderValue::from_str(&retry_after_seconds.to_string()) {
                response.headers_mut().insert(RETRY_AFTER, value);
            }
            return response;
        }
        NarrativeError::GameRulesExhausted => {
            return error_response(
                StatusCode::UNPROCESSABLE_ENTITY,
                "game_rule_generation_exhausted",
                "Game rule template generation budget is exhausted",
            )
        }
        NarrativeError::ReadingProgressBehindWorld => {
            return error_response(
                StatusCode::CONFLICT,
                "reading_progress_behind_world",
                "Read through the world context source chapter before resuming this timeline",
            )
        }
        NarrativeError::Unavailable(error) => {
            tracing::warn!(error = ?error, "novel dependency unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "Novel service is unavailable",
            )
        }
        NarrativeError::Llm(error)
            if error
                .chain()
                .any(|cause| cause.is::<llm_client::NotConfigured>()) =>
        {
            tracing::warn!("narrative LLM configuration is missing");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "llm_not_configured",
                "Configure the language model before retrying the request",
            )
        }
        NarrativeError::Llm(error) => {
            tracing::error!(error = ?error, "narrative LLM operation failed");
            (
                StatusCode::BAD_GATEWAY,
                "llm_error",
                "Consequence generation failed",
            )
        }
        NarrativeError::Internal(error) => {
            tracing::error!(error = ?error, "narrative operation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Narrative operation failed",
            )
        }
    };
    error_response(status, code, message)
}

// ─── Handlers ───────────────────────────────────────────────────────────────

async fn get_player_entry(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
    Query(query): Query<PlayerEntryQuery>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(user_id) = extract_user_id(&headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid user identity",
        );
    };
    match state
        .handler
        .get_player_entry(user_id, novel_id, query.checkpoint_chapter)
        .await
    {
        Ok(entry) => (StatusCode::OK, Json(serde_json::json!(entry))).into_response(),
        Err(error) => narrative_error_response(error),
    }
}

async fn put_player_entry(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
    headers: HeaderMap,
    Json(req): Json<CreatePlayerEntityRequest>,
) -> impl IntoResponse {
    let Some(user_id) = extract_user_id(&headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid user identity",
        );
    };
    let command = CreatePlayerEntityCommand {
        checkpoint_chapter: req.checkpoint_chapter,
        name: req.name,
        background: req.background,
        capabilities: req.capabilities,
        location_id: req.location_id,
        inventory: req.inventory,
        rules: req.rules,
    };
    match state
        .handler
        .create_player_entity(user_id, novel_id, command)
        .await
    {
        Ok(player) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "checkpoint_chapter": player.canonical_checkpoint_chapter,
                "locations": [],
                "player": player,
            })),
        )
            .into_response(),
        Err(error) => narrative_error_response(error),
    }
}

async fn request_game_rule_template(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(user_id) = extract_user_id(&headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid user identity",
        );
    };
    match state
        .handler
        .request_game_rule_template(user_id, novel_id)
        .await
    {
        Ok(template) => (StatusCode::OK, Json(template)).into_response(),
        Err(error) => narrative_error_response(error),
    }
}

/// GET /narrative/:novel_id/chapters/:chapter
async fn get_effective_chapter(
    State(state): State<AppState>,
    Path((novel_id, chapter)): Path<(Uuid, i32)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or invalid user identity",
            );
        }
    };

    match state
        .handler
        .get_effective_chapter(user_id, novel_id, chapter)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))).into_response(),
        Err(error) => narrative_error_response(error),
    }
}

/// GET /narrative/:novel_id/:chapter
async fn get_branch_node(
    State(state): State<AppState>,
    Path((novel_id, chapter)): Path<(Uuid, i32)>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or invalid user identity",
            );
        }
    };

    match state
        .handler
        .get_branch_node(novel_id, chapter, user_id)
        .await
    {
        Ok(Some(node)) => (StatusCode::OK, Json(serde_json::json!(node))).into_response(),
        Ok(None) => error_response(
            StatusCode::NOT_FOUND,
            "not_found",
            "No branch node for this chapter",
        ),
        Err(error) => narrative_error_response(error),
    }
}

/// POST /narrative/choose
async fn submit_choice(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<SubmitChoiceRequest>,
) -> impl IntoResponse {
    let user_id = match extract_user_id(&headers) {
        Some(id) => id,
        None => {
            return error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or invalid user identity",
            );
        }
    };
    match state
        .handler
        .submit_choice(user_id, req.novel_id, req.node_id, req.choice_index)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!(result))).into_response(),
        Err(error) => narrative_error_response(error),
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
            return error_response(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing or invalid user identity",
            );
        }
    };

    match state.handler.get_world_state(user_id, novel_id).await {
        Ok(ws) => (StatusCode::OK, Json(serde_json::json!(ws))).into_response(),
        Err(error) => narrative_error_response(error),
    }
}

async fn start_open_world(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(user_id) = extract_user_id(&headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid user identity",
        );
    };
    match state.handler.start_open_world(user_id, novel_id).await {
        Ok(view) => (StatusCode::OK, Json(view)).into_response(),
        Err(error) => narrative_error_response(error),
    }
}

async fn get_open_world(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(user_id) = extract_user_id(&headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid user identity",
        );
    };
    match state.handler.get_open_world(user_id, novel_id).await {
        Ok(view) => (StatusCode::OK, Json(view)).into_response(),
        Err(error) => narrative_error_response(error),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct WorldTurnRequest {
    expected_turn_number: i64,
    kind: WorldActionKind,
    target_id: Option<String>,
    intent: String,
}

impl WorldTurnRequest {
    fn into_parts(self) -> (i64, WorldAction) {
        (
            self.expected_turn_number,
            WorldAction {
                kind: self.kind,
                target_id: self.target_id,
                intent: self.intent,
            },
        )
    }
}

async fn submit_world_turn(
    State(state): State<AppState>,
    Path(novel_id): Path<Uuid>,
    headers: HeaderMap,
    Json(request): Json<WorldTurnRequest>,
) -> impl IntoResponse {
    let Some(user_id) = extract_user_id(&headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid user identity",
        );
    };
    let turn_id = match extract_idempotency_key(&headers) {
        Ok(turn_id) => turn_id,
        Err(message) => return error_response(StatusCode::BAD_REQUEST, "invalid_request", message),
    };
    let (expected_turn_number, action) = request.into_parts();
    match state
        .handler
        .submit_world_turn(turn_id, user_id, novel_id, expected_turn_number, action)
        .await
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(error) => narrative_error_response(error),
    }
}

/// GET /health
async fn health() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

async fn ready(State(state): State<AppState>) -> impl IntoResponse {
    let (postgres_ready, novel_ready) = dependency_readiness(
        state.postgres_readiness.as_ref(),
        state.novel_readiness.as_ref(),
    )
    .await;
    let status = if postgres_ready && novel_ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    if status == StatusCode::SERVICE_UNAVAILABLE {
        tracing::warn!("narrative-service readiness check failed");
    }
    let body = serde_json::json!({
        "status": if status == StatusCode::OK { "ready" } else { "not_ready" },
        "dependencies": {"postgres": postgres_ready, "novel_service": novel_ready}
    });
    (status, Json(body)).into_response()
}

async fn dependency_readiness(
    postgres: &dyn ReadinessProbe,
    novel: &dyn ReadinessProbe,
) -> (bool, bool) {
    tokio::join!(postgres.is_ready(), novel.is_ready())
}

/// Extract user_id from X-User-Id header; returns None if missing or invalid.
fn extract_user_id(headers: &HeaderMap) -> Option<Uuid> {
    headers
        .get("X-User-Id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| Uuid::parse_str(s).ok())
}

fn extract_idempotency_key(headers: &HeaderMap) -> Result<Uuid, &'static str> {
    let value = headers
        .get("Idempotency-Key")
        .ok_or("Idempotency-Key is required")?
        .to_str()
        .map_err(|_| "Idempotency-Key must be a UUID v4")?;
    let id = Uuid::parse_str(value).map_err(|_| "Idempotency-Key must be a UUID v4")?;
    if id.get_version() != Some(Version::Random)
        || id.get_variant() != Variant::RFC4122
        || id.to_string() != value.to_ascii_lowercase()
    {
        return Err("Idempotency-Key must be a canonical UUID v4");
    }
    Ok(id)
}

fn internal_request_authorized(state: &AppState, headers: &HeaderMap) -> bool {
    internal_token_authorized(headers, state.internal_service_token.as_ref())
}

fn world_context_version(headers: &HeaderMap) -> Option<WorldContextVersion> {
    match headers
        .get(WORLD_CONTEXT_VERSION_HEADER)
        .and_then(|value| value.to_str().ok())
    {
        Some(WORLD_CONTEXT_VERSION_V2) => Some(WorldContextVersion::V2),
        Some(WORLD_CONTEXT_VERSION_V3) => Some(WorldContextVersion::V3),
        _ => None,
    }
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
    fn world_context_version_gate_supports_v2_and_v3_and_fails_closed_otherwise() {
        let mut headers = HeaderMap::new();
        assert_eq!(world_context_version(&headers), None);

        for unsupported in ["1", "4"] {
            headers.insert(
                WORLD_CONTEXT_VERSION_HEADER,
                HeaderValue::from_static(unsupported),
            );
            assert_eq!(world_context_version(&headers), None);
        }

        headers.insert(
            WORLD_CONTEXT_VERSION_HEADER,
            HeaderValue::from_static(WORLD_CONTEXT_VERSION_V2),
        );
        assert_eq!(
            world_context_version(&headers),
            Some(WorldContextVersion::V2)
        );
        headers.insert(
            WORLD_CONTEXT_VERSION_HEADER,
            HeaderValue::from_static(WORLD_CONTEXT_VERSION_V3),
        );
        assert_eq!(
            world_context_version(&headers),
            Some(WorldContextVersion::V3)
        );
    }

    #[test]
    fn absent_character_world_context_is_a_no_content_response() {
        let response = character_world_context_response(Ok(None));
        assert_eq!(response.status(), StatusCode::NO_CONTENT);

        let response = character_context_envelope_response(Ok(None));
        assert_eq!(response.status(), StatusCode::NO_CONTENT);
    }

    #[test]
    fn caller_supplied_user_id_is_rejected() {
        assert!(
            serde_json::from_value::<SubmitChoiceRequest>(serde_json::json!({
                "user_id": Uuid::new_v4(),
                "novel_id": Uuid::new_v4(),
                "node_id": Uuid::new_v4(),
                "choice_index": 0
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CreatePlayerEntityRequest>(serde_json::json!({
                "user_id": Uuid::new_v4(),
                "name": "云舟",
                "background": "来自边城的地图学徒。",
                "capabilities": ["辨认古地图"],
                "location_id": "north-tower",
                "inventory": []
            }))
            .is_err()
        );
    }

    #[tokio::test]
    async fn choice_conflict_uses_the_stable_gateway_error_envelope() {
        let response = narrative_error_response(NarrativeError::ChoiceConflict);
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({
                "error": {
                    "code": "choice_conflict",
                    "message": "A different choice is already committed"
                }
            })
        );
    }

    #[tokio::test]
    async fn llm_errors_distinguish_missing_configuration_from_provider_outage() {
        let not_configured =
            narrative_error_response(NarrativeError::Llm(llm_client::NotConfigured.into()));
        assert_eq!(not_configured.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(not_configured.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"]["code"],
            "llm_not_configured"
        );

        let outage =
            narrative_error_response(NarrativeError::Llm(anyhow::anyhow!("provider timeout")));
        assert_eq!(outage.status(), StatusCode::BAD_GATEWAY);
    }

    #[test]
    fn world_turn_requires_a_strict_client_revision() {
        assert!(
            serde_json::from_value::<WorldTurnRequest>(serde_json::json!({
                "kind": "pursue_goal",
                "target_id": null,
                "intent": "继续追查"
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<WorldTurnRequest>(serde_json::json!({
                "expected_turn_number": 3,
                "kind": "pursue_goal",
                "target_id": null,
                "intent": "继续追查",
                "user_id": Uuid::new_v4()
            }))
            .is_err()
        );

        let request = serde_json::from_value::<WorldTurnRequest>(serde_json::json!({
            "expected_turn_number": 3,
            "kind": "pursue_goal",
            "target_id": null,
            "intent": "继续追查"
        }))
        .unwrap();
        let (expected_turn_number, action) = request.into_parts();
        assert_eq!(expected_turn_number, 3);
        assert_eq!(action.kind, WorldActionKind::PursueGoal);
        assert_eq!(action.intent, "继续追查");
    }

    #[tokio::test]
    async fn stale_branch_tab_after_world_start_uses_conflict_envelope() {
        let response = narrative_error_response(NarrativeError::Conflict(
            "Use world actions after entering the open world".into(),
        ));
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({
                "error": {
                    "code": "conflict",
                    "message": "Use world actions after entering the open world"
                }
            })
        );
    }

    #[tokio::test]
    async fn lost_world_turn_lease_preserves_the_idempotency_contract() {
        let response = narrative_error_response(NarrativeError::TurnOutcomeUnknown);
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({
                "error": {
                    "code": "turn_outcome_unknown",
                    "message": "World turn outcome is unknown; retry with the same Idempotency-Key"
                }
            })
        );
    }

    #[tokio::test]
    async fn game_rule_generation_errors_preserve_retry_and_budget_contracts() {
        let in_progress = narrative_error_response(NarrativeError::GameRulesInProgress {
            retry_after_seconds: 5,
        });
        assert_eq!(in_progress.status(), StatusCode::CONFLICT);
        assert_eq!(
            in_progress.headers().get(RETRY_AFTER).unwrap(),
            HeaderValue::from_static("5"),
        );
        let body = axum::body::to_bytes(in_progress.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({
                "error": {
                    "code": "game_rule_generation_in_progress",
                    "message": "Game rule template generation is in progress"
                }
            }),
        );

        let exhausted = narrative_error_response(NarrativeError::GameRulesExhausted);
        assert_eq!(exhausted.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let body = axum::body::to_bytes(exhausted.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            serde_json::json!({
                "error": {
                    "code": "game_rule_generation_exhausted",
                    "message": "Game rule template generation budget is exhausted"
                }
            }),
        );
    }

    #[tokio::test]
    async fn rewind_hides_world_content_behind_a_stable_conflict_code() {
        let response = narrative_error_response(NarrativeError::ReadingProgressBehindWorld);
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["error"]["code"],
            "reading_progress_behind_world"
        );
        assert!(!String::from_utf8_lossy(&body).contains("world_state"));
    }

    #[test]
    fn routes_construct_with_axum_08_syntax() {
        let _ = routes();
    }

    #[test]
    fn world_turn_idempotency_key_requires_canonical_uuid_v4() {
        let mut headers = HeaderMap::new();
        assert!(extract_idempotency_key(&headers).is_err());
        headers.insert("Idempotency-Key", "not-a-uuid".parse().unwrap());
        assert!(extract_idempotency_key(&headers).is_err());
        headers.insert("Idempotency-Key", Uuid::nil().to_string().parse().unwrap());
        assert!(extract_idempotency_key(&headers).is_err());
        headers.insert(
            "Idempotency-Key",
            "00000000-0000-4000-0000-000000000000".parse().unwrap(),
        );
        assert!(extract_idempotency_key(&headers).is_err());
        let id = Uuid::new_v4();
        headers.insert("Idempotency-Key", id.to_string().parse().unwrap());
        assert_eq!(extract_idempotency_key(&headers), Ok(id));
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

    #[tokio::test]
    async fn dependency_readiness_fails_closed() {
        assert_eq!(
            dependency_readiness(&FixedProbe(true), &FixedProbe(false)).await,
            (true, false)
        );
    }
}
