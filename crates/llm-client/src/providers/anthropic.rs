use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use super::LlmProvider;
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
                let role = if msg.role == "assistant" { "assistant" } else { "user" };
                msgs.push(AnthropicMessage {
                    role: role.to_string(),
                    content: msg.content.clone(),
                });
            }
        }

        (system, msgs)
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn base_url(&self) -> &str {
        ANTHROPIC_API_URL
    }

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

        let resp: AnthropicResponse = client
            .post(format!("{}/v1/messages", ANTHROPIC_API_URL))
            .header("x-api-key", api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        let content = resp.content.first()
            .map(|c| c.text.clone())
            .ok_or_else(|| anyhow!("Empty response"))?;

        Ok(ChatResponse {
            content,
            model: resp.model,
            usage: Some(Usage {
                input_tokens: resp.usage.input_tokens,
                output_tokens: resp.usage.output_tokens,
            }),
        })
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<Box<dyn futures::Stream<Item = Result<String>> + Send + Unpin>> {
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

        let stream = response.bytes_stream().map(|chunk| {
            let chunk = chunk.map_err(|e| anyhow!(e))?;
            let text = String::from_utf8_lossy(&chunk).to_string();
            let content: String = text.lines()
                .filter(|l| l.starts_with("data: "))
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l[6..]).ok())
                .filter(|v| v["type"] == "content_block_delta")
                .filter_map(|v| v["delta"]["text"].as_str().map(String::from))
                .collect();
            Ok(content)
        });

        Ok(Box::new(stream))
    }

    async fn embed(
        &self,
        _client: &reqwest::Client,
        _api_key: &str,
        _request: &EmbeddingRequest,
    ) -> Result<EmbeddingResponse> {
        Err(anyhow!("Anthropic does not support embeddings. Use OpenAI or Gemini."))
    }
}
