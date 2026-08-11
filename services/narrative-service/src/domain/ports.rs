use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn chat(&self, system: &str, user: &str) -> Result<String>;
    async fn chat_json(&self, prompt: &str) -> Result<String>;
}

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
    async fn is_ready(&self) -> bool;
}
