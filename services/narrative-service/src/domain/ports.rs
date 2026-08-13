use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;
use uuid::Uuid;

#[derive(Debug)]
pub struct AccountExportRecord {
    pub kind: String,
    pub data: serde_json::Value,
}

pub type AccountExportStream =
    Pin<Box<dyn futures::Stream<Item = Result<AccountExportRecord>> + Send>>;

pub trait AccountExportPort: Send + Sync {
    fn export_user(&self, user_id: Uuid) -> AccountExportStream;
}

#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn chat_longform(&self, system: &str, user: &str) -> Result<String>;
    async fn chat_json(&self, task: NarrativeLlmTask, prompt: &str) -> Result<String>;
}

#[derive(Debug, Clone, Copy)]
pub enum NarrativeLlmTask {
    BranchGeneration,
    NarrativeTransition,
}

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
    async fn is_ready(&self) -> bool;
}
