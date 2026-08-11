use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

pub trait AccessTokenIssuer: Send + Sync {
    fn generate_token(&self, user_id: Uuid, email: &str, role: &str) -> Result<String>;
}

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
    async fn is_ready(&self) -> bool;
}
