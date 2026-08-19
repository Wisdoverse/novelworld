use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::ports::AgentMemoryPort;

const AGENT_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);

/// HTTP adapter that writes permanent memories on agent-service.
/// Only the journey-memory producer endpoint is exposed.
pub struct AgentServiceClient {
    client: Client,
    base_url: String,
    internal_service_token: String,
}

impl AgentServiceClient {
    pub fn new(base_url: String, internal_service_token: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(AGENT_SERVICE_TIMEOUT)
                .build()
                .expect("valid agent-service HTTP client configuration"),
            base_url,
            internal_service_token,
        }
    }
}

#[async_trait]
impl AgentMemoryPort for AgentServiceClient {
    async fn save_permanent_memory(
        &self,
        memory_id: Uuid,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        event: &str,
        importance: i32,
    ) -> Result<()> {
        let url = format!("{}/internal/memories", self.base_url);
        let resp = self
            .client
            .post(&url)
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .json(&serde_json::json!({
                "memory_id": memory_id,
                "character_id": character_id,
                "user_id": user_id,
                "novel_id": novel_id,
                "chapter_number": chapter_number,
                "event": event,
                "importance": importance,
            }))
            .send()
            .await
            .map_err(|error| anyhow!("Failed to reach agent-service at {}: {}", url, error))?;
        if !resp.status().is_success() {
            return Err(anyhow!(
                "agent-service returned {} for permanent memory write",
                resp.status()
            ));
        }
        Ok(())
    }
}
