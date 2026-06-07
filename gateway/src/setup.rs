use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};

use crate::AppState;

pub static SETUP_COMPLETE: AtomicBool = AtomicBool::new(false);

#[derive(Serialize)]
struct SetupStatus {
    configured: bool,
}

#[derive(Deserialize)]
pub struct TestLlmRequest {
    provider: String,
    api_key: String,
}

#[derive(Deserialize)]
pub struct InitRequest {
    provider: String,
    api_key: String,
    email: String,
    password: String,
    name: Option<String>,
}

pub async fn get_setup_status() -> impl IntoResponse {
    (StatusCode::OK, Json(SetupStatus {
        configured: SETUP_COMPLETE.load(Ordering::SeqCst),
    }))
}

pub async fn test_llm(Json(req): Json<TestLlmRequest>) -> impl IntoResponse {
    let (base_url, model) = match req.provider.as_str() {
        "openai" => ("https://api.openai.com", "gpt-4.1-nano"),
        "deepseek" => ("https://api.deepseek.com", "deepseek-chat"),
        "qwen" => ("https://dashscope.aliyuncs.com/compatible-mode", "qwen-turbo-latest"),
        "glm" => ("https://open.bigmodel.cn/api/paas", "glm-4-flash-250414"),
        "anthropic" => ("https://api.anthropic.com", "claude-haiku-4-5-20251001"),
        "moonshot" => ("https://api.moonshot.cn", "moonshot-v1-8k"),
        "doubao" => ("https://ark.cn-beijing.volces.com/api/v3", "doubao-1.5-lite-32k"),
        "ollama" => ("http://127.0.0.1:11434", "llama3.2"),
        unknown => {
            return (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": format!("Unknown provider: '{}'", unknown)}))).into_response();
        }
    };

    if req.provider == "anthropic" {
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/messages", base_url))
            .header("x-api-key", &req.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&serde_json::json!({
                "model": model,
                "max_tokens": 10,
                "messages": [{"role": "user", "content": "hi"}]
            }))
            .send()
            .await;

        return match resp {
            Ok(r) if r.status().is_success() => {
                (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
            }
            Ok(r) => {
                let status = r.status().as_u16();
                let body = r.text().await.unwrap_or_default();
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": format!("API returned {}: {}", status, body)}))).into_response()
            }
            Err(e) => {
                (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response()
            }
        };
    }

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/v1/chat/completions", base_url))
        .bearer_auth(&req.api_key)
        .json(&serde_json::json!({
            "model": model,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 10,
        }))
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            (StatusCode::OK, Json(serde_json::json!({"status": "ok"}))).into_response()
        }
        Ok(r) => {
            let status = r.status().as_u16();
            let body = r.text().await.unwrap_or_default();
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": format!("API returned {}: {}", status, body)}))).into_response()
        }
        Err(e) => {
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": e.to_string()}))).into_response()
        }
    }
}

pub async fn init_setup(
    State(state): State<AppState>,
    Json(req): Json<InitRequest>,
) -> impl IntoResponse {
    if SETUP_COMPLETE.load(Ordering::SeqCst) {
        return (StatusCode::CONFLICT, Json(serde_json::json!({"error": "Already configured"}))).into_response();
    }

    let register_body = serde_json::json!({
        "email": req.email,
        "password": req.password,
        "name": req.name,
    });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/auth/register", state.proxy.user_service_url))
        .json(&register_body)
        .send()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            SETUP_COMPLETE.store(true, Ordering::SeqCst);
            let body: serde_json::Value = match r.json().await {
                Ok(b) => b,
                Err(e) => {
                    tracing::error!("Failed to parse registration response: {}", e);
                    return (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": format!("Failed to parse response: {}", e)}))).into_response();
                }
            };
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(r) => {
            let body = r.text().await.unwrap_or_default();
            (StatusCode::BAD_REQUEST, Json(serde_json::json!({"error": body}))).into_response()
        }
        Err(e) => {
            (StatusCode::INTERNAL_SERVER_ERROR, Json(serde_json::json!({"error": e.to_string()}))).into_response()
        }
    }
}
