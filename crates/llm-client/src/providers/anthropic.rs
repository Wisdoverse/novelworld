use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    json_response, response_error,
    sse::{decode_stream, SseFrame},
    LlmProvider,
};
use crate::types::*;

pub struct AnthropicProvider;

const ANTHROPIC_API_URL: &str = "https://api.anthropic.com";
const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Serialize)]
struct AnthropicRequest {
    model: String,
    messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    stream: bool,
}

#[derive(Serialize)]
struct AnthropicMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContent>,
    model: String,
    usage: AnthropicUsage,
}

#[derive(Deserialize)]
struct AnthropicContent {
    text: String,
}

#[derive(Deserialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

impl AnthropicProvider {
    fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<AnthropicMessage>) {
        let mut system = None;
        let mut msgs = Vec::new();

        for msg in messages {
            if msg.role == "system" {
                match &mut system {
                    None => system = Some(msg.content.clone()),
                    Some(s) => {
                        s.push('\n');
                        s.push_str(&msg.content);
                    }
                }
            } else {
                let role = if msg.role == "assistant" {
                    "assistant"
                } else {
                    "user"
                };
                msgs.push(AnthropicMessage {
                    role: role.to_string(),
                    content: msg.content.clone(),
                });
            }
        }

        (system, msgs)
    }
}

const KNOWN_STREAM_EVENTS: &[&str] = &[
    "message_start",
    "content_block_start",
    "ping",
    "content_block_delta",
    "content_block_stop",
    "message_delta",
    "message_stop",
    "error",
];

pub(crate) fn parse_stream_frame(frame: SseFrame) -> Result<Vec<ChatStreamEvent>> {
    if frame.event != "message" && !KNOWN_STREAM_EVENTS.contains(&frame.event.as_str()) {
        return Ok(Vec::new());
    }

    let payload: Value = serde_json::from_str(&frame.data)
        .map_err(|error| anyhow!("invalid Anthropic stream payload: {error}"))?;
    let payload_type = payload
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("Anthropic stream payload is missing type"))?;
    let event_type = if frame.event == "message" {
        payload_type
    } else {
        if payload_type != frame.event {
            return Err(anyhow!("Anthropic stream event type mismatch"));
        }
        frame.event.as_str()
    };

    if !KNOWN_STREAM_EVENTS.contains(&event_type) {
        return Ok(Vec::new());
    }

    match event_type {
        "error" => Err(anyhow!("Anthropic stream failed")),
        "message_stop" => Ok(vec![ChatStreamEvent::Finished]),
        "content_block_delta" => {
            let delta = payload
                .get("delta")
                .and_then(Value::as_object)
                .ok_or_else(|| anyhow!("Anthropic content delta is missing delta"))?;
            let delta_type = delta
                .get("type")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Anthropic content delta is missing type"))?;
            if delta_type != "text_delta" {
                return Ok(Vec::new());
            }
            let text = delta
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("Anthropic text delta is missing text"))?;
            if text.is_empty() {
                Ok(Vec::new())
            } else {
                Ok(vec![ChatStreamEvent::Delta(text.to_owned())])
            }
        }
        _ => Ok(Vec::new()),
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn auth_header(&self, api_key: &str) -> (String, String) {
        ("x-api-key".into(), api_key.to_string())
    }

    async fn chat(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<ChatResponse> {
        let (system, messages) = Self::convert_messages(&request.messages);

        let body = AnthropicRequest {
            model: request.model.clone(),
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            stream: false,
        };

        let response = client
            .post(format!("{}/v1/messages", ANTHROPIC_API_URL))
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            return Err(response_error(response).await);
        }

        let resp: AnthropicResponse = json_response(response).await?;

        let content = resp
            .content
            .first()
            .map(|c| c.text.clone())
            .ok_or_else(|| anyhow::anyhow!("Empty response"))?;

        Ok(ChatResponse {
            content,
            model: resp.model,
            usage: Some(Usage::new(
                resp.usage.input_tokens,
                resp.usage.output_tokens,
                None,
            )?),
        })
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<ChatStream> {
        let (system, messages) = Self::convert_messages(&request.messages);

        let body = AnthropicRequest {
            model: request.model.clone(),
            messages,
            system,
            max_tokens: request.max_tokens.unwrap_or(4096),
            temperature: request.temperature,
            stream: true,
        };

        let response = client
            .post(format!("{}/v1/messages", ANTHROPIC_API_URL))
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
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
        _client: &reqwest::Client,
        _api_key: &str,
        _request: &EmbeddingRequest,
    ) -> Result<EmbeddingResponse> {
        Err(anyhow::anyhow!(
            "Anthropic does not support embeddings. Use OpenAI or Gemini."
        ))
    }
}
