pub mod image;

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::domain::ports::LlmPort;

pub struct LlmAdapter {
    client: Arc<llm_client::RuntimeLlmClient>,
}

impl LlmAdapter {
    pub fn new(client: Arc<llm_client::RuntimeLlmClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl LlmPort for LlmAdapter {
    async fn chat(&self, system: &str, user: &str) -> Result<String> {
        self.client.simple_chat(system, user).await
    }

    async fn chat_json(&self, prompt: &str) -> Result<String> {
        self.client.json_chat(prompt).await
    }
}
