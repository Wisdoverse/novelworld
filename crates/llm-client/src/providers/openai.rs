use anyhow::{Result, anyhow};
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use super::LlmProvider;
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

#[async_trait]
impl LlmProvider for OpenAIProvider {
    fn base_url(&self) -> &str {
        &self.base_url
    }

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
        let resp: OpenAIResponse = client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header(&hk, &hv)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        let content = resp.choices.first()
            .and_then(|c| c.message.content.clone())
            .ok_or_else(|| anyhow!("Empty response"))?;

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
    ) -> Result<Box<dyn futures::Stream<Item = Result<String>> + Send + Unpin>> {
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

        let stream = response.bytes_stream().map(|chunk| {
            let chunk = chunk.map_err(|e| anyhow!(e))?;
            let text = String::from_utf8_lossy(&chunk).to_string();
            let content: String = text.lines()
                .filter(|l| l.starts_with("data: ") && !l.contains("[DONE]"))
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l[6..]).ok())
                .filter_map(|v| v["choices"][0]["delta"]["content"].as_str().map(String::from))
                .collect();
            Ok(content)
        });

        Ok(Box::new(stream))
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
        let resp: OpenAIEmbeddingResponse = client
            .post(format!("{}/v1/embeddings", self.base_url))
            .header(&hk, &hv)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;

        let embedding = resp.data.first()
            .map(|d| d.embedding.clone())
            .ok_or_else(|| anyhow!("No embedding returned"))?;

        Ok(EmbeddingResponse { embedding, model: resp.model })
    }
}
