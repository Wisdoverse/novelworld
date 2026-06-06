use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::domain::ports::EmbeddingGenerator;

/// HTTP client that calls an OpenAI-compatible /v1/embeddings endpoint.
pub struct EmbeddingClient {
    client: Client,
    api_url: String,
    api_key: String,
    model: String,
}

impl EmbeddingClient {
    pub fn new(api_url: String, api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_url,
            api_key,
            model,
        }
    }
}

#[derive(Serialize)]
struct EmbeddingRequest {
    model: String,
    input: String,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

#[async_trait]
impl EmbeddingGenerator for EmbeddingClient {
    async fn generate_embedding(&self, text: &str) -> Result<Vec<f32>> {
        let req = EmbeddingRequest {
            model: self.model.clone(),
            input: text.to_string(),
        };
        let resp = self
            .client
            .post(format!("{}/v1/embeddings", self.api_url))
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Embedding API returned HTTP {}: {}",
                status,
                body
            ));
        }

        let resp: EmbeddingResponse = resp.json().await?;
        resp.data
            .first()
            .map(|d| d.embedding.clone())
            .ok_or_else(|| anyhow!("No embedding returned from API"))
    }
}
