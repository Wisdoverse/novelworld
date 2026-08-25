use anyhow::Result;
use async_trait::async_trait;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::ports::PrivacyCleanupPort;

pub struct AgentPrivacyClient {
    base_url: String,
    internal_service_token: String,
    client: reqwest::Client,
}

impl AgentPrivacyClient {
    pub fn new(base_url: String, internal_service_token: String) -> Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            internal_service_token,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(5))
                .build()?,
        })
    }
}

#[async_trait]
impl PrivacyCleanupPort for AgentPrivacyClient {
    async fn clear_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        let response = self
            .client
            .delete(format!(
                "{}/internal/privacy/users/{user_id}/novels/{novel_id}",
                self.base_url
            ))
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("agent privacy cleanup returned {}", response.status());
        }
        Ok(())
    }

    async fn allow_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        let response = self
            .client
            .delete(format!(
                "{}/internal/privacy/tombstones/users/{user_id}/novels/{novel_id}",
                self.base_url
            ))
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .send()
            .await?;
        if !response.status().is_success() {
            anyhow::bail!("agent privacy rollback returned {}", response.status());
        }
        Ok(())
    }
}
