use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};

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
            client: Client::new(),
            api_url,
            api_key,
            model,
        }
    }

    pub async fn generate(&self, prompt: &str) -> Result<String> {
        let req = ImageRequest {
            model: self.model.clone(),
            prompt: prompt.to_string(),
            n: 1,
            size: "1024x1024".into(),
            response_format: "url".into(),
        };
        let resp: ImageResponse = self.client
            .post(format!("{}/v1/images/generations", self.api_url))
            .bearer_auth(&self.api_key)
            .json(&req)
            .send()
            .await?
            .json()
            .await?;
        Ok(resp.data[0].url.clone())
    }
}
