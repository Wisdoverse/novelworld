use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use std::sync::Arc;

use crate::domain::ports::{ChatCompletion, ChatCompletionEvent, ChatStream, TextSummarizer};

pub struct LlmAdapter {
    client: Arc<llm_client::RuntimeLlmClient>,
}

impl LlmAdapter {
    pub fn new(client: Arc<llm_client::RuntimeLlmClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ChatCompletion for LlmAdapter {
    async fn chat_stream(&self, messages: Vec<(String, String)>) -> Result<ChatStream> {
        let mut req = llm_client::ChatRequest::new("");
        for (role, content) in messages {
            req = req.message(&role, content);
        }
        req = req.temperature(0.85).max_tokens(1024);
        Ok(Box::pin(self.client.chat_stream(req).await?.map(|event| {
            event.map(|event| match event {
                llm_client::ChatStreamEvent::Delta(text) => ChatCompletionEvent::Delta(text),
                llm_client::ChatStreamEvent::Finished => ChatCompletionEvent::Finished,
            })
        })))
    }

    async fn chat_messages(&self, messages: Vec<(String, String)>) -> Result<String> {
        let msgs: Vec<llm_client::ChatMessage> = messages
            .into_iter()
            .map(|(role, content)| llm_client::ChatMessage { role, content })
            .collect();
        let req = llm_client::ChatRequest::new("")
            .messages(msgs)
            .temperature(0.85)
            .max_tokens(1024);
        self.client.chat(req).await.map(|r| r.content)
    }
}

#[async_trait]
impl TextSummarizer for LlmAdapter {
    async fn summarize(&self, system: &str, text: &str) -> Result<String> {
        let request = llm_client::ChatRequest::new("")
            .message("system", system)
            .message("user", text)
            .temperature(0.3)
            .max_tokens(256);
        self.client
            .chat(request)
            .await
            .map(|response| response.content)
    }
}
