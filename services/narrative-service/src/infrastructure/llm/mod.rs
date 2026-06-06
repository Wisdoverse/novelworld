use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
    temperature: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
struct Choice {
    message: Message,
}

const MAX_RETRIES: u32 = 3;
const RETRY_DELAYS: [u64; 3] = [1, 2, 4];

pub struct LlmClient {
    client: Client,
    api_url: String,
    api_key: String,
    model: String,
}

impl LlmClient {
    pub fn new(api_url: String, api_key: String, model: String) -> Self {
        Self {
            client: Client::new(),
            api_url,
            api_key,
            model,
        }
    }

    async fn send_with_retry(&self, req: &ChatRequest) -> Result<String> {
        for attempt in 0..MAX_RETRIES {
            let response = self.client
                .post(format!("{}/v1/chat/completions", self.api_url))
                .bearer_auth(&self.api_key)
                .json(req)
                .send()
                .await;

            match response {
                Ok(resp) => {
                    let status = resp.status().as_u16();
                    if status == 429 || status >= 500 {
                        if attempt < MAX_RETRIES - 1 {
                            let delay = if status == 429 {
                                resp.headers()
                                    .get("retry-after")
                                    .and_then(|v| v.to_str().ok())
                                    .and_then(|v| v.parse::<u64>().ok())
                                    .unwrap_or(RETRY_DELAYS[attempt as usize])
                            } else {
                                RETRY_DELAYS[attempt as usize]
                            };
                            tracing::warn!("LLM {} retry {}/{} in {}s", status, attempt + 1, MAX_RETRIES, delay);
                            tokio::time::sleep(Duration::from_secs(delay)).await;
                            continue;
                        }
                        return Err(anyhow!("LLM failed after {} retries: HTTP {}", MAX_RETRIES, status));
                    }
                    let chat_resp: ChatResponse = resp.json().await?;
                    return Ok(chat_resp.choices.first()
                        .ok_or_else(|| anyhow!("Empty LLM response"))?
                        .message.content.clone());
                }
                Err(e) => {
                    if attempt < MAX_RETRIES - 1 {
                        tracing::warn!("LLM error retry {}/{}: {}", attempt + 1, MAX_RETRIES, e);
                        tokio::time::sleep(Duration::from_secs(RETRY_DELAYS[attempt as usize])).await;
                        continue;
                    }
                    return Err(anyhow!("LLM failed after {} retries: {}", MAX_RETRIES, e));
                }
            }
        }
        unreachable!()
    }

    pub async fn chat(&self, system: &str, user: &str) -> Result<String> {
        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                Message { role: "system".into(), content: system.into() },
                Message { role: "user".into(), content: user.into() },
            ],
            response_format: None,
            temperature: 0.8,
        };
        self.send_with_retry(&req).await
    }

    pub async fn chat_json(&self, prompt: &str) -> Result<String> {
        let req = ChatRequest {
            model: self.model.clone(),
            messages: vec![
                Message {
                    role: "system".into(),
                    content: "You are a helpful assistant that always responds with valid JSON.".into(),
                },
                Message { role: "user".into(), content: prompt.into() },
            ],
            response_format: Some(ResponseFormat { format_type: "json_object".into() }),
            temperature: 0.3,
        };
        self.send_with_retry(&req).await
    }
}
