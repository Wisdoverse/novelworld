use axum::{
    extract::{DefaultBodyLimit, Json, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::application::handlers::{
    CreatePlayerEntityCommand, NarrativeCommandHandler, NarrativeError,
};
use crate::domain::ports::ReadinessProbe;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<NarrativeCommandHandler>,
    pub postgres_readiness: Arc<dyn ReadinessProbe>,
    pub novel_readiness: Arc<dyn ReadinessProbe>,
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
        .route("/narrative/{novel_id}/{chapter}", get(get_branch_node))
        .route("/narrative/choose", post(submit_choice))
        .route("/narrative/{novel_id}/world-state", get(get_world_state))
        .route("/health", get(health))
        .route("/ready", get(ready))
        .layer(DefaultBodyLimit::max(4 * 1024))
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
    pub name: String,
    pub background: String,
    pub capabilities: Vec<String>,
    pub location_id: String,
    pub inventory: Vec<String>,
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
        NarrativeError::Unavailable(error) => {
            tracing::warn!(error = ?error, "novel dependency unavailable");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "service_unavailable",
                "Novel service is unavailable",
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
    headers: HeaderMap,
) -> impl IntoResponse {
    let Some(user_id) = extract_user_id(&headers) else {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid user identity",
        );
    };
    match state.handler.get_player_entry(user_id, novel_id).await {
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
        name: req.name,
        background: req.background,
        capabilities: req.capabilities,
        location_id: req.location_id,
        inventory: req.inventory,
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

    #[test]
    fn routes_construct_with_axum_08_syntax() {
        let _ = routes();
    }

    #[tokio::test]
    async fn dependency_readiness_fails_closed() {
        assert_eq!(
            dependency_readiness(&FixedProbe(true), &FixedProbe(false)).await,
            (true, false)
        );
    }
}
