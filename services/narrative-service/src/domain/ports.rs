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

/// Producer port for permanent memories on agent-service (the journey layer:
/// committed world turns become durable, semantically retrievable memories).
#[async_trait]
pub trait AgentMemoryPort: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    async fn save_permanent_memory(
        &self,
        memory_id: Uuid,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        event: &str,
        importance: i32,
    ) -> Result<()>;
}

/// Server-owned entropy port. Implementations must return the same result for
/// the same committed turn identity so retries cannot reroll an outcome.
pub trait DiceRollerPort: Send + Sync {
    fn roll_d20(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        expected_turn_number: i64,
        request_fingerprint: &[u8; 32],
    ) -> u8;
}
