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

    fn is_deepseek(&self) -> bool {
        let is_deepseek = reqwest::Url::parse(&self.base_url)
            .ok()
            .and_then(|url| url.host_str().map(str::to_owned))
            .is_some_and(|host| host == "api.deepseek.com");
        is_deepseek
    }

    fn thinking_control(&self, requested: Option<bool>) -> Option<serde_json::Value> {
        self.is_deepseek().then(|| {
            serde_json::json!({
                "type": if requested.unwrap_or(false) { "enabled" } else { "disabled" }
            })
        })
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
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
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

#[derive(Serialize)]
struct ResponsesRequest {
    model: String,
    input: Vec<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_output_tokens: Option<u32>,
    stream: bool,
    reasoning: serde_json::Value,
}

fn parse_responses_stream_frame(frame: SseFrame) -> Result<Vec<ChatStreamEvent>> {
    let payload: Value = serde_json::from_str(&frame.data)
        .map_err(|error| anyhow!("invalid Responses API stream payload: {error}"))?;
    let event_type = payload
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(frame.event.as_str());
    match event_type {
        "response.output_text.delta" => payload
            .get("delta")
            .and_then(Value::as_str)
            .filter(|delta| !delta.is_empty())
            .map(|delta| vec![ChatStreamEvent::Delta(delta.to_owned())])
            .ok_or_else(|| anyhow!("Responses API text delta is missing")),
        "response.completed" => Ok(vec![ChatStreamEvent::Finished]),
        "response.incomplete" => Err(anyhow!("Responses API output was incomplete")),
        "response.failed" => {
            let message = payload
                .pointer("/response/error/message")
                .or_else(|| payload.pointer("/error/message"))
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            Err(anyhow!("Responses API failed: {message}"))
        }
        _ => Ok(Vec::new()),
    }
}

fn response_content(response: &OpenAIResponse) -> Result<String> {
    response
        .choices
        .first()
        .and_then(|choice| choice.message.content.clone())
        .filter(|content| !content.trim().is_empty())
        .ok_or_else(|| anyhow!("LLM returned an empty response"))
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
        if self.is_deepseek() && request.thinking == Some(true) && !request.json_mode {
            let body = ResponsesRequest {
                model: request.model.clone(),
                input: request.messages.clone(),
                temperature: request.temperature,
                max_output_tokens: Some(
                    request
                        .max_tokens
                        .unwrap_or(1_024)
                        .saturating_add(4_096)
                        .min(8_192),
                ),
                stream: false,
                reasoning: serde_json::json!({"effort": "high"}),
            };
            let (hk, hv) = self.auth_header(api_key);
            let response = client
                .post(format!("{}/v1/responses", self.base_url))
                .header(&hk, &hv)
                .json(&body)
                .send()
                .await?;
            if !response.status().is_success() {
                return Err(response_error(response).await);
            }
            let payload: Value = response.json().await?;
            if payload.get("status").and_then(Value::as_str) != Some("completed") {
                return Err(anyhow!("Responses API output was not completed"));
            }
            let content = payload
                .get("output")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|item| item.get("type").and_then(Value::as_str) == Some("message"))
                .filter_map(|item| item.get("content").and_then(Value::as_array))
                .flatten()
                .filter(|part| part.get("type").and_then(Value::as_str) == Some("output_text"))
                .filter_map(|part| part.get("text").and_then(Value::as_str))
                .collect::<String>();
            if content.trim().is_empty() {
                return Err(anyhow!("Responses API returned an empty output_text"));
            }
            return Ok(ChatResponse {
                content,
                model: payload
                    .get("model")
                    .and_then(Value::as_str)
                    .unwrap_or(&request.model)
                    .to_owned(),
                usage: payload.get("usage").map(|usage| Usage {
                    input_tokens: usage
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or_default() as u32,
                    output_tokens: usage
                        .get("output_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or_default() as u32,
                }),
            });
        }

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
            thinking: self.thinking_control(request.thinking),
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

        let content = match response_content(&resp) {
            Ok(content) => content,
            Err(error) if request.json_mode => {
                // DeepSeek documents that JSON mode can occasionally return an
                // empty content field. Retrying the identical JSON-mode request
                // does not change that failure condition, so make one compatible
                // request without response_format while retaining the explicit
                // JSON-only system prompt.
                tracing::warn!(
                    "JSON mode returned empty content; retrying without response_format"
                );
                let fallback_body = OpenAIRequest {
                    model: request.model.clone(),
                    messages: request.messages.clone(),
                    temperature: request.temperature,
                    max_tokens: request.max_tokens,
                    stream: false,
                    response_format: None,
                    thinking: self.thinking_control(request.thinking),
                };
                let fallback_response = client
                    .post(format!("{}/v1/chat/completions", self.base_url))
                    .header(&hk, &hv)
                    .json(&fallback_body)
                    .send()
                    .await?;
                if !fallback_response.status().is_success() {
                    return Err(response_error(fallback_response).await);
                }
                let fallback: OpenAIResponse = fallback_response.json().await?;
                response_content(&fallback).map_err(|_| error)?
            }
            Err(error) => return Err(error),
        };

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
        if self.is_deepseek() && request.thinking == Some(true) {
            let body = ResponsesRequest {
                model: request.model.clone(),
                input: request.messages.clone(),
                temperature: request.temperature,
                max_output_tokens: Some(
                    request
                        .max_tokens
                        .unwrap_or(1_024)
                        .saturating_add(4_096)
                        .min(8_192),
                ),
                stream: true,
                reasoning: serde_json::json!({"effort": "high"}),
            };
            let (hk, hv) = self.auth_header(api_key);
            let response = client
                .post(format!("{}/v1/responses", self.base_url))
                .header(&hk, &hv)
                .json(&body)
                .send()
                .await?;
            if !response.status().is_success() {
                return Err(response_error(response).await);
            }
            return Ok(decode_stream(
                response.bytes_stream(),
                parse_responses_stream_frame,
            ));
        }

        let body = OpenAIRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: true,
            response_format: None,
            thinking: self.thinking_control(request.thinking),
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

#[cfg(test)]
mod response_tests {
    use super::*;

    #[test]
    fn empty_success_content_is_an_error_so_the_client_can_retry() {
        let response: OpenAIResponse = serde_json::from_str(
            r#"{"choices":[{"message":{"content":"   "}}],"model":"test","usage":null}"#,
        )
        .unwrap();
        assert_eq!(
            response_content(&response).unwrap_err().to_string(),
            "LLM returned an empty response"
        );
    }

    #[test]
    fn deepseek_requests_disable_default_thinking_mode() {
        assert_eq!(
            OpenAIProvider::new(Some("https://api.deepseek.com"))
                .thinking_control(None)
                .unwrap(),
            serde_json::json!({"type": "disabled"})
        );
        assert!(OpenAIProvider::new(Some("https://api.openai.com"))
            .thinking_control(Some(true))
            .is_none());
        assert_eq!(
            OpenAIProvider::new(Some("https://api.deepseek.com"))
                .thinking_control(Some(true))
                .unwrap(),
            serde_json::json!({"type": "enabled"})
        );
    }

    #[test]
    fn responses_stream_emits_only_output_text_and_terminal_state() {
        assert_eq!(
            parse_responses_stream_frame(SseFrame {
                event: "response.output_text.delta".into(),
                data: r#"{"type":"response.output_text.delta","delta":"你好"}"#.into(),
            })
            .unwrap(),
            vec![ChatStreamEvent::Delta("你好".into())]
        );
        assert!(parse_responses_stream_frame(SseFrame {
            event: "response.reasoning_text.delta".into(),
            data: r#"{"type":"response.reasoning_text.delta","delta":"internal"}"#.into(),
        })
        .unwrap()
        .is_empty());
        assert_eq!(
            parse_responses_stream_frame(SseFrame {
                event: "response.completed".into(),
                data: r#"{"type":"response.completed"}"#.into(),
            })
            .unwrap(),
            vec![ChatStreamEvent::Finished]
        );
    }
}
