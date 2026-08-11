pub mod anthropic;
pub mod gemini;
pub mod openai;
pub(crate) mod sse;

use crate::types::{
    ChatRequest, ChatResponse, ChatStream, EmbeddingRequest, EmbeddingResponse, LlmApiError,
};
use anyhow::Result;
use async_trait::async_trait;
use reqwest::header::RETRY_AFTER;

pub(crate) async fn response_error(response: reqwest::Response) -> anyhow::Error {
    let status = response.status().as_u16();
    let retry_after = response
        .headers()
        .get(RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let message = response.text().await.unwrap_or_default();
    LlmApiError {
        status,
        message,
        retry_after,
    }
    .into()
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
