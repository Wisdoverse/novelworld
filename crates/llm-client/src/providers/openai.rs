use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    response_error,
    sse::{decode_stream, SseFrame},
    LlmProvider,
};
use crate::types::*;

pub struct OpenAIProvider {
    base_url: String,
}

impl OpenAIProvider {
    pub fn new(base_url: Option<&str>) -> Self {
        Self {
            base_url: base_url.unwrap_or("https://api.openai.com").to_string(),
        }
    }
}

#[derive(Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
    model: String,
    usage: Option<OpenAIUsage>,
}

#[derive(Deserialize)]
struct OpenAIChoice {
    message: OpenAIMessage,
}

#[derive(Deserialize)]
struct OpenAIMessage {
    content: Option<String>,
}

#[derive(Deserialize)]
struct OpenAIUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
    model: String,
}

#[derive(Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
}

pub(crate) fn parse_stream_frame(frame: SseFrame) -> Result<Vec<ChatStreamEvent>> {
    if frame.event == "error" {
        return Err(anyhow!("OpenAI stream error: {}", frame.data));
    }
    if frame.event != "message" {
        return Ok(Vec::new());
    }
    if frame.data == "[DONE]" {
        return Ok(vec![ChatStreamEvent::Finished]);
    }

    let payload: Value = serde_json::from_str(&frame.data)
        .map_err(|error| anyhow!("invalid OpenAI stream payload: {error}"))?;
    if let Some(error) = payload.get("error") {
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(anyhow!("OpenAI stream error: {message}"));
    }

    let choices = payload
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("OpenAI stream payload is missing choices"))?;
    let mut events = Vec::new();

    for choice in choices {
        let choice = choice
            .as_object()
            .ok_or_else(|| anyhow!("OpenAI stream choice is not an object"))?;

        if let Some(reason) = choice.get("finish_reason") {
            match reason {
                Value::Null => {}
                Value::String(reason) if reason == "content_filter" => {
                    return Err(anyhow!("OpenAI stream was blocked by content filtering"));
                }
                Value::String(_) => {}
                _ => return Err(anyhow!("OpenAI finish_reason is not a string or null")),
            }
        }

        let delta = choice
            .get("delta")
            .and_then(Value::as_object)
            .ok_or_else(|| anyhow!("OpenAI stream choice is missing delta"))?;
        match delta.get("content") {
            Some(Value::String(content)) if !content.is_empty() => {
                events.push(ChatStreamEvent::Delta(content.clone()));
            }
            Some(Value::String(_) | Value::Null) | None => {}
            Some(_) => return Err(anyhow!("OpenAI delta content is not a string or null")),
        }
    }

    Ok(events)
}

#[async_trait]
impl LlmProvider for OpenAIProvider {
    fn auth_header(&self, api_key: &str) -> (String, String) {
        ("Authorization".into(), format!("Bearer {}", api_key))
    }

    async fn chat(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<ChatResponse> {
        let body = OpenAIRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: false,
            response_format: if request.json_mode {
                Some(serde_json::json!({"type": "json_object"}))
            } else {
                None
            },
        };

        let (hk, hv) = self.auth_header(api_key);
        let response = client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header(&hk, &hv)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(response_error(response).await);
        }

        let resp: OpenAIResponse = response.json().await?;

        let content = resp
            .choices
            .first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| anyhow::anyhow!("Empty response"))?;

        Ok(ChatResponse {
            content,
            model: resp.model,
            usage: resp.usage.map(|u| Usage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
            }),
        })
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<ChatStream> {
        let body = OpenAIRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: true,
            response_format: None,
        };

        let (hk, hv) = self.auth_header(api_key);
        let response = client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header(&hk, &hv)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(response_error(response).await);
        }

        Ok(decode_stream(response.bytes_stream(), parse_stream_frame))
    }

    async fn embed(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &EmbeddingRequest,
    ) -> Result<EmbeddingResponse> {
        let body = serde_json::json!({
            "model": request.model,
            "input": request.input,
        });

        let (hk, hv) = self.auth_header(api_key);
        let response = client
            .post(format!("{}/v1/embeddings", self.base_url))
            .header(&hk, &hv)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(response_error(response).await);
        }

        let resp: OpenAIEmbeddingResponse = response.json().await?;

        let embedding = resp
            .data
            .first()
            .map(|d| d.embedding.clone())
            .ok_or_else(|| anyhow::anyhow!("No embedding returned"))?;

        Ok(EmbeddingResponse {
            embedding,
            model: resp.model,
        })
    }
}
