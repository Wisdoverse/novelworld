use async_trait::async_trait;
use anyhow::Result;

#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn chat(&self, system: &str, user: &str) -> Result<String>;
    async fn chat_json(&self, prompt: &str) -> Result<String>;
}
