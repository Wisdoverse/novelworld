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

pub struct GeminiProvider;

const GEMINI_API_URL: &str = "https://generativelanguage.googleapis.com";

#[derive(Serialize)]
struct GeminiRequest {
    contents: Vec<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<GeminiContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    generation_config: Option<GeminiGenConfig>,
}

#[derive(Serialize)]
struct GeminiContent {
    #[serde(skip_serializing_if = "Option::is_none")]
    role: Option<String>,
    parts: Vec<GeminiPart>,
}

#[derive(Serialize)]
struct GeminiPart {
    text: String,
}

#[derive(Serialize)]
struct GeminiGenConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_mime_type: Option<String>,
}

#[derive(Deserialize)]
struct GeminiResponse {
    candidates: Vec<GeminiCandidate>,
    #[serde(default)]
    usage_metadata: Option<GeminiUsage>,
}

#[derive(Deserialize)]
struct GeminiCandidate {
    content: GeminiResponseContent,
}

#[derive(Deserialize)]
struct GeminiResponseContent {
    parts: Vec<GeminiResponsePart>,
}

#[derive(Deserialize)]
struct GeminiResponsePart {
    text: String,
}

#[derive(Deserialize)]
struct GeminiUsage {
    #[serde(default)]
    prompt_token_count: u32,
    #[serde(default)]
    candidates_token_count: u32,
}

#[derive(Deserialize)]
struct GeminiEmbedResponse {
    embedding: GeminiEmbedValues,
}

#[derive(Deserialize)]
struct GeminiEmbedValues {
    values: Vec<f32>,
}

impl GeminiProvider {
    fn convert_messages(messages: &[ChatMessage]) -> (Option<GeminiContent>, Vec<GeminiContent>) {
        let mut system = None;
        let mut contents = Vec::new();

        for msg in messages {
            if msg.role == "system" {
                let part = GeminiPart {
                    text: msg.content.clone(),
                };
                match &mut system {
                    None => {
                        system = Some(GeminiContent {
                            role: None,
                            parts: vec![part],
                        })
                    }
                    Some(s) => s.parts.push(part),
                }
            } else {
                let role = if msg.role == "assistant" {
                    "model"
                } else {
                    "user"
                };
                contents.push(GeminiContent {
                    role: Some(role.to_string()),
                    parts: vec![GeminiPart {
                        text: msg.content.clone(),
                    }],
                });
            }
        }

        (system, contents)
    }
}

pub(crate) fn parse_stream_frame(frame: SseFrame) -> Result<Vec<ChatStreamEvent>> {
    if frame.event == "error" {
        return Err(anyhow!("Gemini stream failed"));
    }
    if frame.event != "message" {
        return Ok(Vec::new());
    }

    let payload: Value = serde_json::from_str(&frame.data)
        .map_err(|error| anyhow!("invalid Gemini stream payload: {error}"))?;
    if payload.get("error").is_some() {
        return Err(anyhow!("Gemini stream failed"));
    }

    if let Some(block_reason) = payload
        .get("promptFeedback")
        .and_then(|feedback| feedback.get("blockReason"))
    {
        match block_reason {
            Value::Null => {}
            Value::String(_) => {
                return Err(anyhow!("Gemini prompt was blocked"));
            }
            _ => return Err(anyhow!("Gemini blockReason is not a string or null")),
        }
    }

    let candidates = payload
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Gemini stream payload is missing candidates"))?;
    let candidate = candidates
        .first()
        .and_then(Value::as_object)
        .ok_or_else(|| anyhow!("Gemini stream payload has no candidate"))?;
    let mut events = Vec::new();

    if let Some(content) = candidate.get("content") {
        let content = content
            .as_object()
            .ok_or_else(|| anyhow!("Gemini candidate content is not an object"))?;
        let parts = content
            .get("parts")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Gemini candidate content is missing parts"))?;
        let mut text = String::new();
        for part in parts {
            let part = part
                .as_object()
                .ok_or_else(|| anyhow!("Gemini candidate part is not an object"))?;
            if let Some(value) = part.get("text") {
                text.push_str(
                    value
                        .as_str()
                        .ok_or_else(|| anyhow!("Gemini candidate text is not a string"))?,
                );
            }
        }
        if !text.is_empty() {
            events.push(ChatStreamEvent::Delta(text));
        }
    }

    match candidate.get("finishReason") {
        Some(Value::String(reason)) if reason == "STOP" || reason == "MAX_TOKENS" => {
            events.push(ChatStreamEvent::Finished);
        }
        Some(Value::String(_)) => {
            return Err(anyhow!("Gemini stream failed"));
        }
        Some(Value::Null) | None => {}
        Some(_) => return Err(anyhow!("Gemini finishReason is not a string or null")),
    }

    Ok(events)
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn auth_header(&self, api_key: &str) -> (String, String) {
        ("x-goog-api-key".into(), api_key.to_string())
    }

    async fn chat(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<ChatResponse> {
        let (system_instruction, contents) = Self::convert_messages(&request.messages);

        let body = GeminiRequest {
            contents,
            system_instruction,
            generation_config: Some(GeminiGenConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
                response_mime_type: if request.json_mode {
                    Some("application/json".into())
                } else {
                    None
                },
            }),
        };

        let url = format!(
            "{}/v1beta/models/{}:generateContent?key={}",
            GEMINI_API_URL, request.model, api_key
        );

        let response = client.post(&url).json(&body).send().await?;

        if !response.status().is_success() {
            return Err(response_error(response).await);
        }

        let resp: GeminiResponse = json_response(response).await?;

        let content = resp
            .candidates
            .first()
            .and_then(|c| c.content.parts.first())
            .map(|p| p.text.clone())
            .ok_or_else(|| anyhow::anyhow!("Empty response"))?;

        Ok(ChatResponse {
            content,
            model: request.model.clone(),
            usage: resp
                .usage_metadata
                .map(|usage| {
                    Usage::new(usage.prompt_token_count, usage.candidates_token_count, None)
                })
                .transpose()?,
        })
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<ChatStream> {
        let (system_instruction, contents) = Self::convert_messages(&request.messages);

        let body = GeminiRequest {
            contents,
            system_instruction,
            generation_config: Some(GeminiGenConfig {
                temperature: request.temperature,
                max_output_tokens: request.max_tokens,
                response_mime_type: None,
            }),
        };

        let url = format!(
            "{}/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
            GEMINI_API_URL, request.model, api_key
        );

        let response = client.post(&url).json(&body).send().await?;

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
        let url = format!(
            "{}/v1beta/models/{}:embedContent?key={}",
            GEMINI_API_URL, request.model, api_key
        );

        let body = serde_json::json!({
            "content": {
                "parts": [{"text": request.input}]
            }
        });

        let response = client.post(&url).json(&body).send().await?;

        if !response.status().is_success() {
            return Err(response_error(response).await);
        }

        let resp: GeminiEmbedResponse = json_response(response).await?;

        Ok(EmbeddingResponse {
            embedding: resp.embedding.values,
            model: request.model.clone(),
        })
    }
}
