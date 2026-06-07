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

    /// Auto-detect providers from environment variables.
    /// Just call `LlmClient::from_env()` — no manual configuration needed.
    ///
    /// Checks these env vars:
    /// - `OPENAI_API_KEY` → OpenAI
    /// - `ANTHROPIC_API_KEY` → Anthropic
    /// - `GEMINI_API_KEY` → Gemini
    /// - `DEEPSEEK_API_KEY` → DeepSeek
    /// - `DOUBAO_API_KEY` → Doubao (CN by default, set `DOUBAO_REGION=intl` for international)
    /// - `QWEN_API_KEY` / `DASHSCOPE_API_KEY` → Qwen (CN by default, set `QWEN_REGION=intl`)
    /// - `GLM_API_KEY` / `ZHIPU_API_KEY` → GLM (CN by default, set `GLM_REGION=intl`)
    /// - `MINIMAX_API_KEY` → MiniMax
    /// - `MOONSHOT_API_KEY` → Moonshot
    /// - `BAICHUAN_API_KEY` → Baichuan
    /// - `STEPFUN_API_KEY` → Stepfun
    /// - `YI_API_KEY` → Yi
    /// - `SPARK_API_KEY` → Spark
    /// - `XIAOMI_API_KEY` → Xiaomi
    /// - `MISTRAL_API_KEY` → Mistral
    /// - `GROQ_API_KEY` → Groq
    /// - `TOGETHER_API_KEY` → Together
    /// - `LLM_API_KEY` + `LLM_API_URL` → Generic OpenAI-compatible fallback
    pub fn from_env() -> Self {
        let mut client = Self::new();

        let env = |key: &str| std::env::var(key).ok();
        let region = |key: &str| env(key).map(|v| v.to_lowercase()).unwrap_or_default();

        if let Some(key) = env("OPENAI_API_KEY") {
            client = client.with_openai(key);
        }
        if let Some(key) = env("ANTHROPIC_API_KEY") {
            client = client.with_anthropic(key);
        }
        if let Some(key) = env("GEMINI_API_KEY") {
            client = client.with_gemini(key);
        }
        if let Some(key) = env("DEEPSEEK_API_KEY") {
            client = client.with_deepseek(key);
        }
        if let Some(key) = env("DOUBAO_API_KEY") {
            client = if region("DOUBAO_REGION") == "intl" {
                client.with_doubao_intl(key)
            } else {
                client.with_doubao_cn(key)
            };
        }
        if let Some(key) = env("QWEN_API_KEY").or_else(|| env("DASHSCOPE_API_KEY")) {
            client = if region("QWEN_REGION") == "intl" {
                client.with_qwen_intl(key)
            } else {
                client.with_qwen_cn(key)
            };
        }
        if let Some(key) = env("GLM_API_KEY").or_else(|| env("ZHIPU_API_KEY")) {
            client = if region("GLM_REGION") == "intl" {
                client.with_glm_intl(key)
            } else {
                client.with_glm_cn(key)
            };
        }
        if let Some(key) = env("MINIMAX_API_KEY") {
            client = client.with_minimax(key);
        }
        if let Some(key) = env("MOONSHOT_API_KEY") {
            client = client.with_moonshot(key);
        }
        if let Some(key) = env("BAICHUAN_API_KEY") {
            client = client.with_baichuan(key);
        }
        if let Some(key) = env("STEPFUN_API_KEY") {
            client = client.with_stepfun(key);
        }
        if let Some(key) = env("YI_API_KEY") {
            client = client.with_yi(key);
        }
        if let Some(key) = env("SPARK_API_KEY") {
            client = client.with_spark(key);
        }
        if let Some(key) = env("XIAOMI_API_KEY") {
            client = client.with_xiaomi(key);
        }
        if let Some(key) = env("MISTRAL_API_KEY") {
            client = client.with_mistral(key);
        }
        if let Some(key) = env("GROQ_API_KEY") {
            client = client.with_groq(key);
        }
        if let Some(key) = env("TOGETHER_API_KEY") {
            client = client.with_together(key);
        }

        // Generic fallback: LLM_API_KEY + LLM_API_URL
        if let Some(key) = env("LLM_API_KEY") {
            let url = env("LLM_API_URL").unwrap_or_else(|| "https://api.openai.com".into());
            client = client.with_openai_compatible("default", key, url);
        }

        // Set default from LLM_PROVIDER env var, or first registered
        if let Some(provider) = env("LLM_PROVIDER") {
            client = client.with_default(provider);
        }

        client
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

    // ─── DeepSeek ────────────────────────────────────────────────────
    pub fn with_deepseek(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("deepseek", api_key, "https://api.deepseek.com")
    }

    // ─── Doubao (ByteDance Volcano Engine) ────────────────────────
    pub fn with_doubao(self, api_key: impl Into<String>) -> Self {
        self.with_doubao_cn(api_key)
    }
    pub fn with_doubao_cn(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("doubao", api_key, "https://ark.cn-beijing.volces.com/api/v3")
    }
    pub fn with_doubao_intl(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("doubao", api_key, "https://ark.ap-southeast.volces.com/api/v3")
    }

    // ─── Qwen (Alibaba Cloud) ─────────────────────────────────────
    pub fn with_qwen(self, api_key: impl Into<String>) -> Self {
        self.with_qwen_cn(api_key)
    }
    pub fn with_qwen_cn(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("qwen", api_key, "https://dashscope.aliyuncs.com/compatible-mode")
    }
    pub fn with_qwen_intl(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("qwen", api_key, "https://dashscope-intl.aliyuncs.com/compatible-mode")
    }

    // ─── MiniMax ──────────────────────────────────────────────────
    pub fn with_minimax(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("minimax", api_key, "https://api.minimax.chat")
    }

    // ─── Xiaomi ───────────────────────────────────────────────────
    pub fn with_xiaomi(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("xiaomi", api_key, "https://api.xiaomi.com")
    }

    // ─── GLM (ZhipuAI) ───────────────────────────────────────────
    pub fn with_glm(self, api_key: impl Into<String>) -> Self {
        self.with_glm_cn(api_key)
    }
    pub fn with_glm_cn(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("glm", api_key, "https://open.bigmodel.cn/api/paas")
    }
    pub fn with_glm_intl(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("glm", api_key, "https://open.bigmodel.com/api/paas")
    }

    // ─── Moonshot (Kimi) ──────────────────────────────────────────
    pub fn with_moonshot(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("moonshot", api_key, "https://api.moonshot.cn")
    }

    // ─── Baichuan ─────────────────────────────────────────────────
    pub fn with_baichuan(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("baichuan", api_key, "https://api.baichuan-ai.com")
    }

    // ─── Stepfun (阶跃星辰) ──────────────────────────────────────
    pub fn with_stepfun(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("stepfun", api_key, "https://api.stepfun.com")
    }

    // ─── 讯飞星火 (iFlytek Spark) ────────────────────────────────
    pub fn with_spark(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("spark", api_key, "https://spark-api-open.xf-yun.com")
    }

    // ─── Mistral ──────────────────────────────────────────────────
    pub fn with_mistral(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("mistral", api_key, "https://api.mistral.ai")
    }

    // ─── Groq ─────────────────────────────────────────────────────
    pub fn with_groq(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("groq", api_key, "https://api.groq.com/openai")
    }

    // ─── Together AI ──────────────────────────────────────────────
    pub fn with_together(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("together", api_key, "https://api.together.xyz")
    }

    // ─── Local / Self-hosted ──────────────────────────────────────
    pub fn with_ollama(self) -> Self {
        self.with_openai_compatible("ollama", "", "http://localhost:11434")
    }
    pub fn with_vllm(self, base_url: impl Into<String>) -> Self {
        self.with_openai_compatible("vllm", "", base_url)
    }

    pub fn with_yi(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("yi", api_key, "https://api.lingyiwanwu.com")
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
                    let status = e.downcast_ref::<LlmApiError>()
                        .map(|ae| ae.status)
                        .unwrap_or(500);

                    if attempt < RetryPolicy::max_retries() - 1 && RetryPolicy::should_retry(status, attempt) {
                        let delay = RetryPolicy::delay(status, attempt, None);
                        tracing::warn!("LLM error ({}), retry {}/{}: {}", status, attempt + 1, RetryPolicy::max_retries(), e);
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
