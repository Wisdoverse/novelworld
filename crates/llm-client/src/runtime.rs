use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use serde::Deserialize;
use tokio::sync::OnceCell;

use crate::{ChatRequest, ChatResponse, ChatStream, LlmClient};

pub struct RuntimeLlmClient {
    source: ConfigSource,
    resolved: OnceCell<Arc<ResolvedClient>>,
}

enum ConfigSource {
    Static(RuntimeConfig),
    Remote {
        client: reqwest::Client,
        user_service_url: String,
        token: String,
    },
}

struct RuntimeConfig {
    provider: String,
    api_url: String,
    model: String,
    api_key: String,
    thinking_enabled: bool,
}

struct ResolvedClient {
    client: LlmClient,
    model: String,
    thinking_enabled: bool,
}

#[derive(Deserialize)]
struct RemoteConfig {
    contract: u8,
    api_url: String,
    model: String,
    api_key: String,
    thinking_enabled: bool,
}

impl RuntimeLlmClient {
    pub fn from_env() -> Result<Self> {
        let user_service_url =
            std::env::var("USER_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:8001".into());
        let token = std::env::var("INTERNAL_SERVICE_TOKEN").map_err(|_| {
            anyhow!("INTERNAL_SERVICE_TOKEN is required for runtime LLM configuration")
        })?;
        if token.len() < 32 {
            return Err(anyhow!(
                "INTERNAL_SERVICE_TOKEN must be at least 32 characters"
            ));
        }
        Ok(Self::remote(user_service_url, token))
    }

    pub fn static_config(
        api_url: String,
        model: String,
        api_key: String,
        thinking_enabled: bool,
    ) -> Self {
        Self {
            source: ConfigSource::Static(RuntimeConfig {
                provider: provider_for_url(&api_url).into(),
                api_url,
                model,
                api_key,
                thinking_enabled,
            }),
            resolved: OnceCell::new(),
        }
    }

    pub fn remote(user_service_url: String, token: String) -> Self {
        Self {
            source: ConfigSource::Remote {
                client: reqwest::Client::new(),
                user_service_url: user_service_url.trim_end_matches('/').into(),
                token,
            },
            resolved: OnceCell::new(),
        }
    }

    async fn resolved(&self, runtime_user_id: Option<&str>) -> Result<Arc<ResolvedClient>> {
        if let ConfigSource::Remote {
            client,
            user_service_url,
            token,
        } = &self.source
        {
            let response = remote_config_request(client, user_service_url, token, runtime_user_id)
                .send()
                .await?;
            if !response.status().is_success() {
                return Err(anyhow!(
                    "runtime LLM configuration is unavailable ({})",
                    response.status()
                ));
            }
            return Ok(Arc::new(build_resolved(validate_remote_config(
                response.json().await?,
            )?)));
        }

        self.resolved
            .get_or_try_init(|| async {
                let config = match &self.source {
                    ConfigSource::Static(config) => RuntimeConfig {
                        provider: config.provider.clone(),
                        api_url: config.api_url.clone(),
                        model: config.model.clone(),
                        api_key: config.api_key.clone(),
                        thinking_enabled: config.thinking_enabled,
                    },
                    ConfigSource::Remote { .. } => unreachable!(),
                };
                Ok(Arc::new(build_resolved(config)))
            })
            .await
            .cloned()
    }

    pub async fn chat(&self, mut request: ChatRequest) -> Result<ChatResponse> {
        let resolved = self.resolved(request.runtime_user_id.as_deref()).await?;
        request.model.clone_from(&resolved.model);
        if request.thinking.is_none() {
            request.thinking = Some(resolved.thinking_enabled);
        }
        resolved.client.chat(request).await
    }

    pub async fn chat_stream(&self, mut request: ChatRequest) -> Result<ChatStream> {
        let resolved = self.resolved(request.runtime_user_id.as_deref()).await?;
        request.model.clone_from(&resolved.model);
        if request.thinking.is_none() {
            request.thinking = Some(resolved.thinking_enabled);
        }
        resolved.client.chat_stream(request).await
    }

    pub async fn simple_chat(
        &self,
        operation: crate::LlmOperation,
        system: &str,
        user: &str,
    ) -> Result<String> {
        self.chat(
            ChatRequest::new(operation, "")
                .message("system", system)
                .message("user", user)
                .temperature(0.8)
                .max_tokens(1024),
        )
        .await
        .map(|response| response.content)
    }

    /// Generate prose where the output itself is the product. Reasoning mode
    /// is deliberately disabled so providers such as DeepSeek cannot consume
    /// the response budget with hidden reasoning and return an incomplete
    /// chapter instead of usable text.
    pub async fn longform_chat(
        &self,
        operation: crate::LlmOperation,
        system: &str,
        user: &str,
    ) -> Result<String> {
        self.chat(longform_request(operation, system, user))
            .await
            .map(|response| response.content)
    }

    pub async fn longform_chat_for_user(
        &self,
        runtime_user_id: impl Into<String>,
        operation: crate::LlmOperation,
        system: &str,
        user: &str,
    ) -> Result<String> {
        self.chat(longform_request(operation, system, user).runtime_user_id(runtime_user_id))
            .await
            .map(|response| response.content)
    }

    pub async fn json_chat(&self, operation: crate::LlmOperation, prompt: &str) -> Result<String> {
        self.chat(json_request(operation, prompt))
            .await
            .map(|response| response.content)
    }

    pub async fn json_chat_for_user(
        &self,
        runtime_user_id: impl Into<String>,
        operation: crate::LlmOperation,
        prompt: &str,
    ) -> Result<String> {
        self.chat(json_request(operation, prompt).runtime_user_id(runtime_user_id))
            .await
            .map(|response| response.content)
    }
}

fn remote_config_request(
    client: &reqwest::Client,
    user_service_url: &str,
    token: &str,
    runtime_user_id: Option<&str>,
) -> reqwest::RequestBuilder {
    let mut request = client
        .get(format!("{user_service_url}/internal/runtime/llm"))
        .header("X-Internal-Service-Token", token)
        .timeout(Duration::from_secs(5));
    if let Some(user_id) = runtime_user_id {
        request = request.header("X-User-Id", user_id);
    }
    request
}

fn longform_request(operation: crate::LlmOperation, system: &str, user: &str) -> ChatRequest {
    ChatRequest::new(operation, "")
        .message("system", system)
        .message("user", user)
        .temperature(0.8)
        .max_tokens(8_192)
        .thinking(false)
}

fn json_request(operation: crate::LlmOperation, prompt: &str) -> ChatRequest {
    ChatRequest::new(operation, "")
        .message(
            "system",
            "You are a helpful assistant that always responds with a non-empty valid JSON object. Output JSON only.",
        )
        .message("user", prompt)
        .temperature(0.3)
        .max_tokens(operation.max_output_tokens())
        .thinking(false)
        .json()
}

fn build_resolved(config: RuntimeConfig) -> ResolvedClient {
    let model = format!("{}/{}", config.provider, config.model);
    let client =
        LlmClient::new().with_openai_compatible(&config.provider, config.api_key, config.api_url);
    ResolvedClient {
        client,
        model,
        thinking_enabled: config.thinking_enabled,
    }
}

fn validate_remote_config(config: RemoteConfig) -> Result<RuntimeConfig> {
    if config.contract != 2
        || !config.api_url.starts_with("https://")
        || config.model.trim().is_empty()
        || config.model.len() > 200
        || config.api_key.trim().is_empty()
        || config.api_key.len() > 4_096
    {
        return Err(anyhow!("invalid runtime LLM configuration"));
    }
    let provider = provider_for_url(&config.api_url).into();
    Ok(RuntimeConfig {
        provider,
        api_url: config.api_url,
        model: config.model,
        api_key: config.api_key,
        thinking_enabled: config.thinking_enabled,
    })
}

fn provider_for_url(api_url: &str) -> &'static str {
    match reqwest::Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .as_deref()
    {
        Some("api.deepseek.com") => "deepseek",
        Some("api.openai.com") => "openai",
        _ => "environment",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_configuration_fails_closed() {
        assert!(validate_remote_config(RemoteConfig {
            contract: 2,
            api_url: "http://127.0.0.1:11434".into(),
            model: "model".into(),
            api_key: "secret".into(),
            thinking_enabled: false,
        })
        .is_err());
    }

    #[test]
    fn remote_configuration_request_forwards_only_explicit_user_context() {
        let client = reqwest::Client::new();
        let user_request = remote_config_request(
            &client,
            "https://user-service.example",
            "internal-token",
            Some("123e4567-e89b-12d3-a456-426614174000"),
        )
        .build()
        .unwrap();
        assert_eq!(
            user_request
                .headers()
                .get("X-User-Id")
                .unwrap()
                .to_str()
                .unwrap(),
            "123e4567-e89b-12d3-a456-426614174000"
        );

        let platform_request = remote_config_request(
            &client,
            "https://user-service.example",
            "internal-token",
            None,
        )
        .build()
        .unwrap();
        assert!(!platform_request.headers().contains_key("X-User-Id"));
    }
}
