use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;

use crate::domain::ports::TextSummarizer;

pub struct LlmAdapter {
    client: Arc<llm_client::LlmClient>,
    model: String,
}

impl LlmAdapter {
    pub fn new(client: Arc<llm_client::LlmClient>, model: String) -> Self {
        Self { client, model }
    }

    /// Streaming chat used by the handler for SSE endpoints.
    pub async fn chat_stream(
        &self,
        messages: Vec<(String, String)>,
    ) -> Result<Box<dyn futures::Stream<Item = Result<String>> + Send + Unpin>> {
        let mut req = llm_client::ChatRequest::new(&self.model);
        for (role, content) in messages {
            req = req.message(&role, content);
        }
        req = req.temperature(0.85);
        self.client.chat_stream(req).await
    }

    /// Non-streaming multi-message chat used by the handler.
    pub async fn chat_messages(&self, messages: Vec<(String, String)>) -> Result<String> {
        let msgs: Vec<llm_client::ChatMessage> = messages
            .into_iter()
            .map(|(role, content)| llm_client::ChatMessage { role, content })
            .collect();
        let req = llm_client::ChatRequest::new(&self.model)
            .messages(msgs)
            .temperature(0.85);
        self.client.chat(req).await.map(|r| r.content)
    }
}

#[async_trait]
impl TextSummarizer for LlmAdapter {
    async fn summarize(&self, system: &str, text: &str) -> Result<String> {
        self.client.simple_chat(&self.model, system, text).await
    }
}
