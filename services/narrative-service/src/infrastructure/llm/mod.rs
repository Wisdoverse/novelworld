use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

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
    async fn chat_longform(&self, user_id: Uuid, system: &str, user: &str) -> Result<String> {
        self.client
            .longform_chat_for_user(
                user_id.to_string(),
                llm_client::LlmOperation::PlayerChapter,
                system,
                user,
            )
            .await
    }

    async fn chat_json(
        &self,
        user_id: Uuid,
        task: NarrativeLlmTask,
        prompt: &str,
    ) -> Result<String> {
        let operation = match task {
            NarrativeLlmTask::BranchGeneration => llm_client::LlmOperation::BranchGeneration,
            NarrativeLlmTask::NarrativeTransition => llm_client::LlmOperation::NarrativeTransition,
        };
        self.client
            .json_chat_for_user(user_id.to_string(), operation, prompt)
            .await
    }
}
