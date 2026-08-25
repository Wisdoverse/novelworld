use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::entities::{llm_usage::LlmUsageSnapshot, runtime_config::RuntimeLlmConfig};

pub trait AccessTokenIssuer: Send + Sync {
    fn generate_token(&self, user_id: Uuid, email: &str, role: &str) -> Result<String>;
}

#[derive(Debug, thiserror::Error)]
pub enum PasswordHasherError {
    #[error("Password hashing capacity is busy")]
    Capacity,
    #[error("Password operation failed")]
    Internal(#[source] anyhow::Error),
}

#[async_trait]
pub trait PasswordHasher: Send + Sync {
    async fn hash(&self, password: &str) -> std::result::Result<String, PasswordHasherError>;
    async fn verify(
        &self,
        password: &str,
        hash: &str,
    ) -> std::result::Result<bool, PasswordHasherError>;
}

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
    async fn is_ready(&self) -> bool;
}

#[async_trait]
pub trait LlmConnectionTester: Send + Sync {
    async fn test(&self, config: &RuntimeLlmConfig) -> Result<()>;
}

#[async_trait]
pub trait LlmUsageReader: Send + Sync {
    async fn read(&self, config: &RuntimeLlmConfig) -> Result<LlmUsageSnapshot>;
}

#[async_trait]
pub trait PrivacyCleanupPort: Send + Sync {
    async fn clear_user(&self, user_id: Uuid) -> Result<()>;
    async fn allow_user(&self, user_id: Uuid) -> Result<()>;
}
