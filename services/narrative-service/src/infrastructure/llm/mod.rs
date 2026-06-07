use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;

use crate::domain::ports::LlmPort;

pub struct LlmAdapter {
    client: Arc<llm_client::LlmClient>,
    model: String,
}

impl LlmAdapter {
    pub fn new(client: Arc<llm_client::LlmClient>, model: String) -> Self {
        Self { client, model }
    }
}

#[async_trait]
impl LlmPort for LlmAdapter {
    async fn chat(&self, system: &str, user: &str) -> Result<String> {
        self.client.simple_chat(&self.model, system, user).await
    }

    async fn chat_json(&self, prompt: &str) -> Result<String> {
        self.client.json_chat(&self.model, prompt).await
    }
}
