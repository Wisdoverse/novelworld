use anyhow::Result;
use async_trait::async_trait;

use crate::domain::{entities::runtime_config::RuntimeLlmConfig, ports::LlmConnectionTester};

pub struct LlmClientTester;

#[async_trait]
impl LlmConnectionTester for LlmClientTester {
    async fn test(&self, config: &RuntimeLlmConfig) -> Result<()> {
        let client = llm_client::LlmClient::new().with_openai_compatible(
            &config.provider,
            &config.api_key,
            &config.api_url,
        );
        let result = client
            .chat(
                llm_client::ChatRequest::new(
                    llm_client::LlmOperation::SetupConnection,
                    format!("{}/{}", config.provider, config.model),
                )
                .message("user", "Reply OK")
                .max_tokens(8)
                .thinking(false),
            )
            .await;
        connection_result(result)
    }
}

fn connection_result(result: Result<llm_client::ChatResponse>) -> Result<()> {
    match result {
        Ok(_) => Ok(()),
        // A parsed successful completion proves connectivity even when a
        // reasoning model spends this bounded probe's eight output tokens.
        // Diagnostic evidence errors remain terminal and are never accepted.
        Err(error) if error.is::<llm_client::TruncatedCompletion>() => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_accepts_only_parsed_success_or_typed_truncation() {
        assert!(connection_result(Err(
            anyhow::Error::new(llm_client::TruncatedCompletion).context("parsed completion")
        ))
        .is_ok());
        assert!(connection_result(Err(anyhow::anyhow!("LLM response was truncated"))).is_err());
        assert!(connection_result(Err(
            llm_client::diagnostic_budget::BudgetEvidenceError.into()
        ))
        .is_err());
        assert!(connection_result(Err(llm_client::LlmApiError {
            status: 401,
            message: "provider request failed".into(),
            retry_after: None
        }
        .into()))
        .is_err());
    }
}
