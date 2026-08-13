pub mod anthropic;
pub mod gemini;
pub mod openai;
pub(crate) mod sse;

use crate::types::{
    ChatRequest, ChatResponse, ChatStream, EmbeddingRequest, EmbeddingResponse, LlmApiError,
};
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::header::RETRY_AFTER;
use serde::de::DeserializeOwned;

const MAX_JSON_RESPONSE_BYTES: usize = 1024 * 1024;

pub(crate) async fn response_error(response: reqwest::Response) -> anyhow::Error {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    LlmApiError {
        status,
        message: "provider request failed".into(),
        retry_after,
    }
    .into()
}

pub(crate) async fn json_response<T: DeserializeOwned>(response: reqwest::Response) -> Result<T> {
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if body.len().saturating_add(chunk.len()) > MAX_JSON_RESPONSE_BYTES {
            return Err(LlmApiError {
                status: 413,
                message: format!("provider response exceeds {MAX_JSON_RESPONSE_BYTES} bytes"),
                retry_after: None,
            }
            .into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&body)?)
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn auth_header(&self, api_key: &str) -> (String, String);

    async fn chat(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<ChatResponse>;

    async fn chat_stream(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &ChatRequest,
    ) -> Result<ChatStream>;

    async fn embed(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &EmbeddingRequest,
    ) -> Result<EmbeddingResponse>;
}
