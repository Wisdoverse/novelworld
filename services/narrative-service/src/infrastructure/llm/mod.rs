use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::domain::ports::{LlmPort, NarrativeLlmTask};

pub struct LlmAdapter {
    client: Arc<llm_client::RuntimeLlmClient>,
}

impl LlmAdapter {
    pub fn new(client: Arc<llm_client::RuntimeLlmClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl LlmPort for LlmAdapter {
    async fn chat_longform(&self, system: &str, user: &str) -> Result<String> {
        self.client
            .longform_chat(llm_client::LlmOperation::PlayerChapter, system, user)
            .await
    }

    async fn chat_json(&self, task: NarrativeLlmTask, prompt: &str) -> Result<String> {
        let operation = match task {
            NarrativeLlmTask::BranchGeneration => llm_client::LlmOperation::BranchGeneration,
            NarrativeLlmTask::NarrativeTransition => llm_client::LlmOperation::NarrativeTransition,
        };
        self.client.json_chat(operation, prompt).await
    }
}
