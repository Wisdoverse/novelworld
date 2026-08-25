use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::domain::ports::EmbeddingGenerator;

pub struct EmbeddingAdapter {
    client: Arc<llm_client::LlmClient>,
    model: String,
}

pub struct NoopEmbeddingGenerator;

pub fn default_model_for_api(api_url: &str) -> String {
    let url = api_url.to_lowercase();
    if url.contains("openai.com") {
        "text-embedding-3-small".into()
    } else if url.contains("dashscope") {
        "text-embedding-v3".into()
    } else if url.contains("bigmodel.cn") || url.contains("bigmodel.com") {
        "embedding-3".into()
    } else if url.contains("siliconflow") {
        "BAAI/bge-m3".into()
    } else if url.contains("localhost") || url.contains("127.0.0.1") {
        "nomic-embed-text".into()
    } else if url.contains("mistral") {
        "mistral-embed".into()
    } else if url.contains("baichuan") {
        "Baichuan-Text-Embedding".into()
    } else if url.contains("volces.com") {
        "doubao-embedding".into()
    } else {
        "text-embedding-3-small".into()
    }
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

#[async_trait]
impl EmbeddingGenerator for NoopEmbeddingGenerator {
    async fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>> {
        Err(anyhow::anyhow!(
            "Embedding not configured — semantic search disabled"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::default_model_for_api;

    #[test]
    fn provider_defaults_remain_stable() {
        assert_eq!(
            default_model_for_api("https://api.openai.com"),
            "text-embedding-3-small"
        );
        assert_eq!(
            default_model_for_api("http://127.0.0.1:11434"),
            "nomic-embed-text"
        );
        assert_eq!(
            default_model_for_api("https://unknown.example"),
            "text-embedding-3-small"
        );
    }
}
