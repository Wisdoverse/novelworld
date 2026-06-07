use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::domain::ports::EmbeddingGenerator;

pub struct EmbeddingAdapter {
    client: Arc<llm_client::LlmClient>,
    model: String,
}

impl EmbeddingAdapter {
    pub fn new(client: Arc<llm_client::LlmClient>, model: String) -> Self {
        Self { client, model }
    }
}

#[async_trait]
impl EmbeddingGenerator for EmbeddingAdapter {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let req = llm_client::EmbeddingRequest {
            model: self.model.clone(),
            input: text.to_string(),
        };
        self.client.embed(req).await.map(|r| r.embedding)
    }
}
