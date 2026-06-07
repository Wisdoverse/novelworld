use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use serde::{Deserialize, Serialize};

use super::LlmProvider;
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
                let part = GeminiPart { text: msg.content.clone() };
                match &mut system {
                    None => system = Some(GeminiContent { role: None, parts: vec![part] }),
                    Some(s) => s.parts.push(part),
                }
            } else {
                let role = if msg.role == "assistant" { "model" } else { "user" };
                contents.push(GeminiContent {
                    role: Some(role.to_string()),
                    parts: vec![GeminiPart { text: msg.content.clone() }],
                });
            }
        }

        (system, contents)
    }
}

#[async_trait]
impl LlmProvider for GeminiProvider {
    fn base_url(&self) -> &str {
        GEMINI_API_URL
    }

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

        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmApiError { status, message: body }.into());
        }

        let resp: GeminiResponse = response.json().await?;

        let content = resp.candidates.first()
            .and_then(|c| c.content.parts.first())
            .map(|p| p.text.clone())
            .ok_or_else(|| anyhow::anyhow!("Empty response"))?;

        Ok(ChatResponse {
            content,
            model: request.model.clone(),
            usage: resp.usage_metadata.map(|u| Usage {
                input_tokens: u.prompt_token_count,
                output_tokens: u.candidates_token_count,
            }),
        })
    }

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<Box<dyn futures::Stream<Item = Result<String>> + Send + Unpin>> {
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
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmApiError { status, message: body }.into());
        }

        let stream = response.bytes_stream().map(|chunk| {
            let chunk = chunk.map_err(|e| anyhow::anyhow!(e))?;
            let text = String::from_utf8_lossy(&chunk).to_string();
            let content: String = text.lines()
                .filter(|l| l.starts_with("data: "))
                .filter_map(|l| serde_json::from_str::<serde_json::Value>(&l[6..]).ok())
                .filter_map(|v| {
                    v["candidates"][0]["content"]["parts"][0]["text"]
                        .as_str().map(String::from)
                })
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
        let url = format!(
            "{}/v1beta/models/{}:embedContent?key={}",
            GEMINI_API_URL, request.model, api_key
        );

        let body = serde_json::json!({
            "content": {
                "parts": [{"text": request.input}]
            }
        });

        let response = client
            .post(&url)
            .json(&body)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(LlmApiError { status, message: body }.into());
        }

        let resp: GeminiEmbedResponse = response.json().await?;

        Ok(EmbeddingResponse {
            embedding: resp.embedding.values,
            model: request.model.clone(),
        })
    }
}
