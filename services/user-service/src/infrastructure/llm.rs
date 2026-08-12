use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{entities::runtime_config::RuntimeLlmConfig, ports::LlmConnectionTester};

pub struct LlmClientTester;

#[async_trait]
impl LlmConnectionTester for LlmClientTester {
    async fn test(&self, config: &RuntimeLlmConfig) -> Result<()> {
        let client = llm_client::LlmClient::new().with_openai_compatible(
            "setup",
            &config.api_key,
            &config.api_url,
        );
        client
            .chat(
                llm_client::ChatRequest::new(format!("setup/{}", config.model))
                    .message("user", "Reply OK")
                    .max_tokens(8),
            )
            .await?;
        Ok(())
    }
}
