use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize)]
struct Message {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    temperature: f32,
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
            temperature: 0.8,
        };
        self.send_with_retry(&req).await
    }

    pub async fn chat_messages(&self, messages: Vec<(String, String)>) -> Result<String> {
        let msgs: Vec<Message> = messages.into_iter()
            .map(|(role, content)| Message { role, content })
            .collect();
        let req = ChatRequest {
            model: self.model.clone(),
            messages: msgs,
            temperature: 0.85,
        };
        self.send_with_retry(&req).await
    }

    pub async fn chat_stream(
        &self,
        messages: Vec<(String, String)>,
    ) -> Result<impl futures::Stream<Item = Result<String>>> {
        use futures::StreamExt;

        let msgs: Vec<Message> = messages.into_iter()
            .map(|(role, content)| Message { role, content })
            .collect();

        let body = json!({
            "model": self.model,
            "messages": msgs,
            "stream": true,
            "temperature": 0.85,
        });

        let response = self.client
            .post(format!("{}/v1/chat/completions", self.api_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?;

        let stream = response.bytes_stream().map(|chunk| {
            let chunk = chunk?;
            let text = String::from_utf8_lossy(&chunk).to_string();
            let content = text.lines()
                .filter(|l| l.starts_with("data: ") && !l.contains("[DONE]"))
                .filter_map(|l| {
                    let json_str = &l["data: ".len()..];
                    serde_json::from_str::<serde_json::Value>(json_str).ok()
                })
                .filter_map(|v| {
                    v["choices"][0]["delta"]["content"]
                        .as_str()
                        .map(|s| s.to_string())
                })
                .collect::<String>();
            Ok(content)
        });

        Ok(stream)
    }
}
