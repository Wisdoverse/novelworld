use anyhow::{bail, Result};
use async_trait::async_trait;
use futures::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use crate::domain::ports::ImagePort;

#[derive(Debug, Serialize)]
struct ImageRequest {
    model: String,
    prompt: String,
    n: u32,
    size: String,
    response_format: String,
}

#[derive(Debug, Deserialize)]
struct ImageResponse {
    data: Vec<ImageData>,
}

#[derive(Debug, Deserialize)]
struct ImageData {
    url: String,
}

/// 图像生成客户端（OpenAI DALL-E 兼容 API）
pub struct ImageClient {
    client: Client,
    api_url: String,
    api_key: String,
    model: String,
}

impl ImageClient {
    pub fn new(api_url: String, api_key: String, model: String) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(300))
                .build()
                .expect("valid static image HTTP client configuration"),
            api_url,
            api_key,
            model,
        }
    }
}

#[async_trait]
impl ImagePort for ImageClient {
    async fn generate(&self, prompt: &str) -> Result<String> {
        if self.api_key.trim().is_empty() {
            bail!("Image generation is not configured");
        }
        let req = ImageRequest {
            model: self.model.clone(),
            prompt: prompt.to_string(),
            n: 1,
            size: "1024x1024".into(),
            response_format: "url".into(),
        };
        let response = self
            .client
            .post(format!("{}/v1/images/generations", self.api_url))
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?;
        if !response.status().is_success() {
            let status = response.status();
            return Err(anyhow::anyhow!("Image API error {status}"));
        }
        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body.len().saturating_add(chunk.len()) > 1024 * 1024 {
                bail!("Image API response exceeded 1048576 bytes");
            }
            body.extend_from_slice(&chunk);
        }
        let resp: ImageResponse = serde_json::from_slice(&body)?;
        resp.data
            .first()
            .map(|d| d.url.clone())
            .ok_or_else(|| anyhow::anyhow!("Image API returned empty data"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn empty_key_fails_before_outbound_io() {
        let client = ImageClient::new(
            "https://unregistered.invalid".into(),
            String::new(),
            "test".into(),
        );

        assert_eq!(
            client
                .generate("private prompt")
                .await
                .unwrap_err()
                .to_string(),
            "Image generation is not configured"
        );
    }
}
