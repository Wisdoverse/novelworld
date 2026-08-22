pub mod image;

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::domain::ports::{LlmPort, NovelLlmTask};

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
    async fn chat_json(&self, task: NovelLlmTask, prompt: &str) -> Result<String> {
        let operation = match task {
            NovelLlmTask::ChapterBoundaryDetection => {
                llm_client::LlmOperation::ChapterBoundaryDetection
            }
            NovelLlmTask::CharacterExtraction => llm_client::LlmOperation::CharacterExtraction,
            NovelLlmTask::CanonExtraction => llm_client::LlmOperation::CanonExtraction,
            NovelLlmTask::NarrativeNodeDetection => {
                llm_client::LlmOperation::NarrativeNodeDetection
            }
        };
        self.client.json_chat(operation, prompt).await
    }
}
