pub mod openai;
pub mod anthropic;
pub mod gemini;

use anyhow::Result;
use async_trait::async_trait;
use crate::types::{ChatRequest, ChatResponse, EmbeddingRequest, EmbeddingResponse};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    fn base_url(&self) -> &str;
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
    ) -> Result<Box<dyn futures::Stream<Item = Result<String>> + Send + Unpin>>;

    async fn embed(
        &self,
        client: &reqwest::Client,
        api_key: &str,
        request: &EmbeddingRequest,
    ) -> Result<EmbeddingResponse>;
}
