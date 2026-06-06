use axum::{
    extract::{Path, State, Json},
    http::{StatusCode, HeaderMap, header},
    response::{IntoResponse, Sse, sse::Event},
    routing::{get, post},
    Router,
};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;

use crate::application::handlers::AgentCommandHandler;

#[derive(Clone)]
pub struct AppState {
    pub handler: Arc<AgentCommandHandler>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        // 流式对话（SSE）
        .route("/chat/:character_id/stream", post(chat_stream))
        // 普通对话
        .route("/chat/:character_id", post(chat))
        // 获取对话历史
        .route("/chat/:character_id/history", get(get_history))
        // 获取角色记忆
        .route("/memories/:character_id", get(get_memories))
        // 清除短期记忆
        .route("/memories/:character_id/short", axum::routing::delete(clear_short_memory))
        .with_state(state)
}

#[derive(Debug, Deserialize)]
pub struct ChatRequest {
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub message: String,
    pub reader_identity: Option<String>,
    pub current_chapter: i32,
}

#[derive(Debug, Serialize)]
pub struct ChatResponse {
    pub message: String,
    pub character_id: Uuid,
}

/// 流式 SSE 对话接口
async fn chat_stream(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let handler = state.handler.clone();

    let stream = async_stream::stream! {
        match handler.chat_stream(
            character_id,
            req.user_id,
            req.novel_id,
            req.message,
            req.reader_identity,
            req.current_chapter,
        ).await {
            Ok(mut s) => {
                while let Some(chunk) = s.next().await {
                    match chunk {
                        Ok(text) if !text.is_empty() => {
                            yield Ok::<Event, anyhow::Error>(
                                Event::default().data(text)
                            );
                        }
                        Err(e) => {
                            yield Ok(Event::default()
                                .event("error")
                                .data(e.to_string()));
                            break;
                        }
                        _ => {}
                    }
                }
                yield Ok(Event::default().event("done").data(""));
            }
            Err(e) => {
                yield Ok(Event::default()
                    .event("error")
                    .data(e.to_string()));
            }
        }
    };

    Sse::new(stream)
}

/// 普通对话接口（非流式）
async fn chat(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    match state.handler.chat(
        character_id,
        req.user_id,
        req.novel_id,
        req.message,
        req.reader_identity,
        req.current_chapter,
    ).await {
        Ok(response) => (StatusCode::OK, Json(ChatResponse {
            message: response,
            character_id,
        })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({
            "error": e.to_string()
        }))).into_response(),
    }
}

async fn get_history(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
) -> impl IntoResponse {
    // TODO: 从 query params 获取 user_id, novel_id
    (StatusCode::OK, Json(serde_json::json!({ "messages": [] }))).into_response()
}

async fn get_memories(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
) -> impl IntoResponse {
    (StatusCode::OK, Json(serde_json::json!({ "memories": [] }))).into_response()
}

async fn clear_short_memory(
    State(state): State<AppState>,
    Path(character_id): Path<Uuid>,
) -> impl IntoResponse {
    StatusCode::NO_CONTENT.into_response()
}
