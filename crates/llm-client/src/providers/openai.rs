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
    stream_options: Option<StreamOptions>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct StreamOptions {
    include_usage: bool,
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
    #[serde(default)]
    prompt_cache_hit_tokens: Option<u32>,
    #[serde(default)]
    prompt_cache_miss_tokens: Option<u32>,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokenDetails>,
}

#[derive(Deserialize)]
struct PromptTokenDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

impl OpenAIUsage {
    fn into_usage(self) -> Result<Usage> {
        let standard_cached = self
            .prompt_tokens_details
            .and_then(|details| details.cached_tokens);
        let deepseek_cached = match (self.prompt_cache_hit_tokens, self.prompt_cache_miss_tokens) {
            (Some(hit), Some(miss)) if hit.checked_add(miss) == Some(self.prompt_tokens) => {
                Some(hit)
            }
            (Some(_), Some(_)) => return Err(anyhow!("provider returned invalid cache usage")),
            (Some(hit), None) => Some(hit),
            (None, Some(miss)) if miss <= self.prompt_tokens => Some(self.prompt_tokens - miss),
            (None, Some(_)) => return Err(anyhow!("provider returned invalid cache usage")),
            (None, None) => None,
        };
        if deepseek_cached.is_some()
            && standard_cached.is_some()
            && deepseek_cached != standard_cached
        {
            return Err(anyhow!("provider returned conflicting cache usage"));
        }
        Usage::new(
            self.prompt_tokens,
            self.completion_tokens,
            deepseek_cached.or(standard_cached),
        )
    }
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

#[derive(Deserialize)]
struct ResponsesUsage {
    input_tokens: u32,
    output_tokens: u32,
    #[serde(default)]
    input_tokens_details: Option<InputTokenDetails>,
}

#[derive(Deserialize)]
struct InputTokenDetails {
    #[serde(default)]
    cached_tokens: Option<u32>,
}

impl ResponsesUsage {
    fn into_usage(self) -> Result<Usage> {
        Usage::new(
            self.input_tokens,
            self.output_tokens,
            self.input_tokens_details
                .and_then(|details| details.cached_tokens),
        )
    }
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
        "response.completed" => {
            let mut events = Vec::new();
            if let Some(usage) = payload.pointer("/response/usage") {
                events.push(ChatStreamEvent::Usage(
                    serde_json::from_value::<ResponsesUsage>(usage.clone())?.into_usage()?,
                ));
            }
            events.push(ChatStreamEvent::Finished);
            Ok(events)
        }
        "response.incomplete" => Err(anyhow!("Responses API output was incomplete")),
        "response.failed" => Err(anyhow!("Responses API failed")),
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
        return Err(anyhow!("OpenAI stream failed"));
    }
    if frame.event != "message" {
        return Ok(Vec::new());
    }
    if frame.data == "[DONE]" {
        return Ok(vec![ChatStreamEvent::Finished]);
    }

    let payload: Value = serde_json::from_str(&frame.data)
        .map_err(|error| anyhow!("invalid OpenAI stream payload: {error}"))?;
    if payload.get("error").is_some() {
        return Err(anyhow!("OpenAI stream failed"));
    }

    let mut events = Vec::new();
    if let Some(usage) = payload.get("usage").filter(|usage| !usage.is_null()) {
        events.push(ChatStreamEvent::Usage(
            serde_json::from_value::<OpenAIUsage>(usage.clone())?.into_usage()?,
        ));
    }

    let choices = payload
        .get("choices")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("OpenAI stream payload is missing choices"))?;
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
                max_output_tokens: Some(request.effective_max_output_tokens().unwrap_or(1_024)),
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
            let payload: Value = json_response(response).await?;
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
                usage: payload
                    .get("usage")
                    .filter(|usage| !usage.is_null())
                    .map(|usage| {
                        serde_json::from_value::<ResponsesUsage>(usage.clone())?.into_usage()
                    })
                    .transpose()?,
            });
        }

        let body = OpenAIRequest {
            model: request.model.clone(),
            messages: request.messages.clone(),
            temperature: request.temperature,
            max_tokens: request.max_tokens,
            stream: false,
            stream_options: None,
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

        let resp: OpenAIResponse = json_response(response).await?;

        let content = response_content(&resp);
        let usage = resp.usage.map(OpenAIUsage::into_usage).transpose()?;
        let content = match content {
            Ok(content) => content,
            Err(_) if request.json_mode => {
                // DeepSeek documents that JSON mode can occasionally return an
                // empty content field. The shared client owns the single
                // response_format-free fallback so it is counted as an attempt.
                return Err(JsonModeEmpty(usage).into());
            }
            Err(error) => return Err(error),
        };

        Ok(ChatResponse {
            content,
            model: resp.model,
            usage,
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
                max_output_tokens: Some(request.effective_max_output_tokens().unwrap_or(1_024)),
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
            stream_options: Some(StreamOptions {
                include_usage: true,
            }),
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

        let resp: OpenAIEmbeddingResponse = json_response(response).await?;

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
    fn usage_accepts_standard_or_deepseek_cache_fields_and_rejects_conflicts() {
        for payload in [
            r#"{"prompt_tokens":10,"completion_tokens":2}"#,
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":4}"#,
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":4,"prompt_cache_miss_tokens":6}"#,
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_miss_tokens":6}"#,
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_tokens_details":{"cached_tokens":4}}"#,
        ] {
            let usage = serde_json::from_str::<OpenAIUsage>(payload)
                .unwrap()
                .into_usage()
                .unwrap();
            assert!(usage.cached_input_tokens.is_none() || usage.cached_input_tokens == Some(4));
        }
        assert!(serde_json::from_str::<OpenAIUsage>(
            r#"{"prompt_tokens":3,"completion_tokens":2,"prompt_cache_hit_tokens":4}"#,
        )
        .unwrap()
        .into_usage()
        .is_err());
        assert!(serde_json::from_str::<OpenAIUsage>(
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":4,"prompt_cache_miss_tokens":7}"#,
        )
        .unwrap()
        .into_usage()
        .is_err());
        assert!(serde_json::from_str::<OpenAIUsage>(
            r#"{"prompt_tokens":10,"completion_tokens":2,"prompt_cache_hit_tokens":4,"prompt_tokens_details":{"cached_tokens":5}}"#,
        )
        .unwrap()
        .into_usage()
        .is_err());
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
