use anyhow::{Result, anyhow};
use std::collections::HashMap;

use crate::providers::LlmProvider;
use crate::providers::openai::OpenAIProvider;
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::gemini::GeminiProvider;
use crate::retry::RetryPolicy;
use crate::types::*;

pub struct LlmClient {
    http: reqwest::Client,
    providers: HashMap<String, (Box<dyn LlmProvider>, String)>,
    default_provider: Option<String>,
}

impl LlmClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
            providers: HashMap::new(),
            default_provider: None,
        }
    }

    pub fn with_openai(mut self, api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        self.providers.insert(
            "openai".into(),
            (Box::new(OpenAIProvider::new(None)), key),
        );
        if self.default_provider.is_none() {
            self.default_provider = Some("openai".into());
        }
        self
    }

    pub fn with_anthropic(mut self, api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        self.providers.insert(
            "anthropic".into(),
            (Box::new(AnthropicProvider), key),
        );
        if self.default_provider.is_none() {
            self.default_provider = Some("anthropic".into());
        }
        self
    }

    pub fn with_gemini(mut self, api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        self.providers.insert(
            "gemini".into(),
            (Box::new(GeminiProvider), key),
        );
        if self.default_provider.is_none() {
            self.default_provider = Some("gemini".into());
        }
        self
    }

    pub fn with_openai_compatible(
        mut self,
        name: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        let n = name.into();
        let key = api_key.into();
        let url = base_url.into();
        self.providers.insert(
            n.clone(),
            (Box::new(OpenAIProvider::new(Some(&url))), key),
        );
        if self.default_provider.is_none() {
            self.default_provider = Some(n);
        }
        self
    }

    pub fn with_deepseek(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("deepseek", api_key, "https://api.deepseek.com")
    }

    pub fn with_doubao(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("doubao", api_key, "https://ark.cn-beijing.volces.com/api/v3")
    }

    pub fn with_qwen(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("qwen", api_key, "https://dashscope.aliyuncs.com/compatible-mode")
    }

    pub fn with_minimax(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("minimax", api_key, "https://api.minimax.chat")
    }

    pub fn with_xiaomi(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("xiaomi", api_key, "https://api.xiaomi.com")
    }

    pub fn with_default(mut self, provider: impl Into<String>) -> Self {
        self.default_provider = Some(provider.into());
        self
    }

    fn resolve_provider(&self, model: &str) -> Result<(&dyn LlmProvider, &str, String)> {
        if let Some(idx) = model.find('/') {
            let provider_name = &model[..idx];
            let model_name = &model[idx + 1..];
            let (provider, api_key) = self.providers.get(provider_name)
                .ok_or_else(|| anyhow!("Unknown provider: {}. Available: {:?}", provider_name, self.providers.keys().collect::<Vec<_>>()))?;
            Ok((provider.as_ref(), api_key, model_name.to_string()))
        } else if let Some(default) = &self.default_provider {
            let (provider, api_key) = self.providers.get(default)
                .ok_or_else(|| anyhow!("Default provider '{}' not configured", default))?;
            Ok((provider.as_ref(), api_key, model.to_string()))
        } else {
            Err(anyhow!("No provider specified in model '{}' and no default set", model))
        }
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        let (provider, api_key, model_name) = self.resolve_provider(&request.model)?;
        let mut req = request;
        req.model = model_name;

        for attempt in 0..RetryPolicy::max_retries() {
            match provider.chat(&self.http, api_key, &req).await {
                Ok(resp) => return Ok(resp),
                Err(e) => {
                    if attempt < RetryPolicy::max_retries() - 1 {
                        let delay = RetryPolicy::delay(500, attempt, None);
                        tracing::warn!("LLM chat error, retry {}/{}: {}", attempt + 1, RetryPolicy::max_retries(), e);
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        unreachable!()
    }

    pub async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Box<dyn futures::Stream<Item = Result<String>> + Send + Unpin>> {
        let (provider, api_key, model_name) = self.resolve_provider(&request.model)?;
        let mut req = request;
        req.model = model_name;
        req.stream = true;
        provider.chat_stream(&self.http, api_key, &req).await
    }

    pub async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        let (provider, api_key, model_name) = self.resolve_provider(&request.model)?;
        let req = EmbeddingRequest { model: model_name, input: request.input };
        provider.embed(&self.http, api_key, &req).await
    }

    pub async fn simple_chat(
        &self,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<String> {
        let request = ChatRequest::new(model)
            .message("system", system)
            .message("user", user)
            .temperature(0.8);
        self.chat(request).await.map(|r| r.content)
    }

    pub async fn json_chat(
        &self,
        model: &str,
        prompt: &str,
    ) -> Result<String> {
        let request = ChatRequest::new(model)
            .message("system", "You are a helpful assistant that always responds with valid JSON.")
            .message("user", prompt)
            .temperature(0.3)
            .json();
        self.chat(request).await.map(|r| r.content)
    }
}
