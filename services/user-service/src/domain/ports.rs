use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::entities::runtime_config::RuntimeLlmConfig;

pub trait AccessTokenIssuer: Send + Sync {
    fn generate_token(&self, user_id: Uuid, email: &str, role: &str) -> Result<String>;
}

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
    async fn is_ready(&self) -> bool;
}

#[async_trait]
pub trait LlmConnectionTester: Send + Sync {
    async fn test(&self, config: &RuntimeLlmConfig) -> Result<()>;
}
