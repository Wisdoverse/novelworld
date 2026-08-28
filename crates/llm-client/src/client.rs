use anyhow::{anyhow, Result};
use async_stream::stream;
use futures::StreamExt;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant as TokioInstant,
};

use crate::providers::anthropic::AnthropicProvider;
use crate::providers::gemini::GeminiProvider;
use crate::providers::openai::OpenAIProvider;
use crate::providers::LlmProvider;
use crate::retry::RetryPolicy;
use crate::telemetry::RequestLabels;
use crate::types::*;

pub struct LlmClient {
    http: reqwest::Client,
    providers: HashMap<String, (Box<dyn LlmProvider>, String)>,
    default_provider: Option<String>,
    admission: Arc<Semaphore>,
}

const MAX_CONCURRENT_LLM_REQUESTS: usize = 8;
const LLM_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(not(test))]
const LLM_TOTAL_TIMEOUT: Duration = Duration::from_secs(300);
#[cfg(test)]
const LLM_TOTAL_TIMEOUT: Duration = Duration::from_millis(250);

fn shared_admission() -> Arc<Semaphore> {
    static ADMISSION: OnceLock<Arc<Semaphore>> = OnceLock::new();
    ADMISSION
        .get_or_init(|| Arc::new(Semaphore::new(MAX_CONCURRENT_LLM_REQUESTS)))
        .clone()
}

impl Default for LlmClient {
    fn default() -> Self {
        Self::new()
    }
}

impl LlmClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::builder()
                .connect_timeout(LLM_CONNECT_TIMEOUT)
                .timeout(LLM_TOTAL_TIMEOUT)
                .build()
                .expect("valid static LLM HTTP client configuration"),
            providers: HashMap::new(),
            default_provider: None,
            admission: shared_admission(),
        }
    }

    fn admit(&self) -> Result<OwnedSemaphorePermit> {
        self.admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow!("LLM request capacity is busy"))
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
        if let Some(key) = env("OPENROUTER_API_KEY") {
            client = client.with_openrouter(key);
        }
        if let Some(key) = env("SILICONFLOW_API_KEY") {
            client = client.with_siliconflow(key);
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
        self.providers
            .insert("openai".into(), (Box::new(OpenAIProvider::new(None)), key));
        if self.default_provider.is_none() {
            self.default_provider = Some("openai".into());
        }
        self
    }

    pub fn with_anthropic(mut self, api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        self.providers
            .insert("anthropic".into(), (Box::new(AnthropicProvider), key));
        if self.default_provider.is_none() {
            self.default_provider = Some("anthropic".into());
        }
        self
    }

    pub fn with_gemini(mut self, api_key: impl Into<String>) -> Self {
        let key = api_key.into();
        self.providers
            .insert("gemini".into(), (Box::new(GeminiProvider), key));
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
        self.providers
            .insert(n.clone(), (Box::new(OpenAIProvider::new(Some(&url))), key));
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
        self.with_openai_compatible(
            "doubao",
            api_key,
            "https://ark.cn-beijing.volces.com/api/v3",
        )
    }
    pub fn with_doubao_intl(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible(
            "doubao",
            api_key,
            "https://ark.ap-southeast.volces.com/api/v3",
        )
    }

    // ─── Qwen (Alibaba Cloud) ─────────────────────────────────────
    pub fn with_qwen(self, api_key: impl Into<String>) -> Self {
        self.with_qwen_cn(api_key)
    }
    pub fn with_qwen_cn(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible(
            "qwen",
            api_key,
            "https://dashscope.aliyuncs.com/compatible-mode",
        )
    }
    pub fn with_qwen_intl(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible(
            "qwen",
            api_key,
            "https://dashscope-intl.aliyuncs.com/compatible-mode",
        )
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

    // ─── OpenRouter ────────────────────────────────────────────
    pub fn with_openrouter(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("openrouter", api_key, "https://openrouter.ai/api")
    }

    // ─── SiliconFlow (硅基流动) ─────────────────────────────────
    pub fn with_siliconflow(self, api_key: impl Into<String>) -> Self {
        self.with_openai_compatible("siliconflow", api_key, "https://api.siliconflow.cn")
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

    fn resolve_provider(&self, model: &str) -> Result<(&dyn LlmProvider, &str, String, String)> {
        if let Some(idx) = model.find('/') {
            let provider_name = &model[..idx];
            let model_name = &model[idx + 1..];
            let (provider, api_key) = self.providers.get(provider_name).ok_or_else(|| {
                anyhow!(
                    "Unknown provider: {}. Available: {:?}",
                    provider_name,
                    self.providers.keys().collect::<Vec<_>>()
                )
            })?;
            Ok((
                provider.as_ref(),
                api_key,
                provider_name.to_string(),
                model_name.to_string(),
            ))
        } else if let Some(default) = &self.default_provider {
            let (provider, api_key) = self
                .providers
                .get(default)
                .ok_or_else(|| anyhow!("Default provider '{}' not configured", default))?;
            Ok((
                provider.as_ref(),
                api_key,
                default.clone(),
                model.to_string(),
            ))
        } else {
            Err(anyhow!(
                "No provider specified in model '{}' and no default set",
                model
            ))
        }
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<ChatResponse> {
        validate_request(&request)?;
        let started = Instant::now();
        let (provider, api_key, provider_name, model_name) =
            self.resolve_provider(&request.model)?;
        let _permit = self.admit()?;
        let labels = RequestLabels::new(
            &provider_name,
            &model_name,
            api_key,
            request.operation,
            "sync",
            request.effective_max_output_tokens().unwrap(),
        );
        labels.started();
        let mut req = request;
        req.model = model_name;

        let deadline = TokioInstant::now() + LLM_TOTAL_TIMEOUT;
        match tokio::time::timeout_at(deadline, async {
            let mut retry_attempt = 0;
            loop {
                let attempt_started = Instant::now();
                match provider.chat(&self.http, api_key, &req).await {
                    Ok(resp) => {
                        labels.attempt("success", attempt_started.elapsed().as_secs_f64());
                        if provider.reports_response_model() {
                            labels.response_model(&resp.model);
                        }
                        labels.usage(resp.usage.as_ref());
                        labels.finish("success", started);
                        return Ok(resp);
                    }
                    Err(e) => {
                        if req.json_mode && e.downcast_ref::<JsonModeEmpty>().is_some() {
                            if let Some(usage) = e
                                .downcast_ref::<JsonModeEmpty>()
                                .and_then(|empty| empty.0.as_ref())
                            {
                                labels.additional_usage(usage);
                            }
                            labels.attempt(
                                "empty_json_mode",
                                attempt_started.elapsed().as_secs_f64(),
                            );
                            labels.retry("json_mode_fallback");
                            req.json_mode = false;
                            continue;
                        }
                        let api_error = e.downcast_ref::<LlmApiError>();
                        let status = api_error.map(|error| error.status).unwrap_or(500);
                        let metric_status = error_status(api_error);
                        labels.attempt(metric_status, attempt_started.elapsed().as_secs_f64());

                        if RetryPolicy::should_retry(status, retry_attempt) {
                            labels.retry(metric_status);
                            let delay = RetryPolicy::delay(
                                status,
                                retry_attempt,
                                api_error.and_then(|error| error.retry_after.as_deref()),
                            );
                            retry_attempt += 1;
                            tracing::warn!(
                                "LLM error ({}), retry {}/{}: {}",
                                status,
                                retry_attempt,
                                RetryPolicy::max_retries(),
                                e
                            );
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        labels.finish("error", started);
                        return Err(e);
                    }
                }
            }
        })
        .await
        {
            Ok(result) => result,
            Err(_) => {
                labels.finish("timeout", started);
                Err(anyhow!("LLM request exceeded the total deadline"))
            }
        }
    }

    pub async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream> {
        validate_request(&request)?;
        let started = Instant::now();
        let (provider, api_key, provider_name, model_name) =
            self.resolve_provider(&request.model)?;
        let permit = self.admit()?;
        let labels = RequestLabels::new(
            &provider_name,
            &model_name,
            api_key,
            request.operation,
            "stream",
            request.effective_max_output_tokens().unwrap(),
        );
        labels.started();
        let mut req = request;
        req.model = model_name;
        req.stream = true;

        let deadline = TokioInstant::now() + LLM_TOTAL_TIMEOUT;
        let upstream = match tokio::time::timeout_at(deadline, async {
            for attempt in 0..=RetryPolicy::max_retries() {
                let attempt_started = Instant::now();
                match provider.chat_stream(&self.http, api_key, &req).await {
                    Ok(upstream) => {
                        let setup = attempt_started.elapsed().as_secs_f64();
                        labels.attempt("success", setup);
                        labels.stream_setup("success", setup);
                        return Ok(upstream);
                    }
                    Err(error) => {
                        let api_error = error.downcast_ref::<LlmApiError>();
                        let status = api_error.map(|error| error.status).unwrap_or(500);
                        let metric_status = error_status(api_error);
                        let setup = attempt_started.elapsed().as_secs_f64();
                        labels.attempt(metric_status, setup);
                        labels.stream_setup(metric_status, setup);

                        if RetryPolicy::should_retry(status, attempt) {
                            labels.retry(metric_status);
                            let delay = RetryPolicy::delay(
                                status,
                                attempt,
                                api_error.and_then(|error| error.retry_after.as_deref()),
                            );
                            tracing::warn!(
                                "LLM stream setup error ({}), retry {}/{}: {}",
                                status,
                                attempt + 1,
                                RetryPolicy::max_retries(),
                                error
                            );
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        labels.finish("setup_error", started);
                        return Err(error);
                    }
                }
            }
            unreachable!()
        })
        .await
        {
            Ok(result) => result?,
            Err(_) => {
                labels.finish("setup_timeout", started);
                return Err(anyhow!("LLM request exceeded the total deadline"));
            }
        };
        Ok(observe_stream(upstream, labels, started, permit, deadline))
    }

    pub async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        let (provider, api_key, _, model_name) = self.resolve_provider(&request.model)?;
        let _permit = self.admit()?;
        let req = EmbeddingRequest {
            model: model_name,
            input: request.input,
        };
        tokio::time::timeout(LLM_TOTAL_TIMEOUT, provider.embed(&self.http, api_key, &req))
            .await
            .map_err(|_| anyhow!("LLM request exceeded the total deadline"))?
    }

    pub async fn simple_chat(
        &self,
        operation: LlmOperation,
        model: &str,
        system: &str,
        user: &str,
    ) -> Result<String> {
        let request = ChatRequest::new(operation, model)
            .message("system", system)
            .message("user", user)
            .temperature(0.8)
            .max_tokens(1024);
        self.chat(request).await.map(|r| r.content)
    }

    pub async fn json_chat(
        &self,
        operation: LlmOperation,
        model: &str,
        prompt: &str,
    ) -> Result<String> {
        let request = ChatRequest::new(operation, model)
            .message(
                "system",
                "You are a helpful assistant that always responds with a non-empty valid JSON object. Output JSON only.",
            )
            .message("user", prompt)
            .temperature(0.3)
            .max_tokens(operation.max_output_tokens())
            .json();
        self.chat(request).await.map(|r| r.content)
    }
}

fn validate_request(request: &ChatRequest) -> Result<()> {
    let max_tokens = request
        .effective_max_output_tokens()
        .ok_or_else(|| anyhow!("LLM request must declare an output-token limit"))?;
    if max_tokens == 0 || max_tokens > request.operation.max_output_tokens() {
        return Err(anyhow!(
            "LLM operation {} allows at most {} output tokens",
            request.operation.to_str(),
            request.operation.max_output_tokens()
        ));
    }
    Ok(())
}

fn error_status(error: Option<&LlmApiError>) -> &'static str {
    match error.map(|error| error.status) {
        Some(429) => "rate_limited",
        Some(500..) => "provider_error",
        Some(_) => "rejected",
        None => "client_or_transport_error",
    }
}

struct StreamGuard {
    labels: RequestLabels,
    started: Instant,
    first_token: bool,
    response_model: Option<String>,
    usage: Option<Usage>,
    terminal: bool,
    _permit: OwnedSemaphorePermit,
}

impl StreamGuard {
    fn finish(&mut self, status: &'static str) {
        if self.terminal {
            return;
        }
        if self.usage.is_some() || status == "success" {
            self.labels.usage(self.usage.as_ref());
        }
        self.labels.finish(status, self.started);
        self.terminal = true;
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        self.finish("consumer_dropped");
    }
}

fn observe_stream(
    mut upstream: ChatStream,
    labels: RequestLabels,
    started: Instant,
    permit: OwnedSemaphorePermit,
    deadline: TokioInstant,
) -> ChatStream {
    let mut guard = StreamGuard {
        labels,
        started,
        first_token: false,
        response_model: None,
        usage: None,
        terminal: false,
        _permit: permit,
    };

    Box::pin(stream! {
        loop {
            let item = match tokio::time::timeout_at(deadline, upstream.next()).await {
                Ok(item) => item,
                Err(_) => {
                    guard.finish("timeout");
                    yield Err(anyhow!("LLM request exceeded the total deadline"));
                    return;
                }
            };
            let Some(item) = item else {
                guard.finish("stream_error");
                yield Err(anyhow!("LLM stream ended without a terminal event"));
                return;
            };
            match item {
                Ok(ChatStreamEvent::Delta(text)) => {
                    if !guard.first_token && !text.is_empty() {
                        guard.labels.first_token(guard.started);
                        guard.first_token = true;
                    }
                    yield Ok(ChatStreamEvent::Delta(text));
                }
                Ok(ChatStreamEvent::ResponseModel(model)) => {
                    match guard.response_model.as_deref() {
                        None => guard.response_model = Some(model),
                        Some(current) if current == model => {}
                        Some(_) => {
                            guard.finish("stream_error");
                            yield Err(anyhow!("LLM response model changed during the stream"));
                            return;
                        }
                    }
                }
                Ok(ChatStreamEvent::Usage(usage)) => {
                    if guard.usage.is_some()
                        || usage.cached_input_tokens.is_some_and(|cached| cached > usage.input_tokens)
                    {
                        guard.finish("stream_error");
                        yield Err(anyhow!("invalid or duplicate LLM stream usage"));
                        return;
                    }
                    guard.usage = Some(usage);
                }
                Ok(ChatStreamEvent::Finished) => {
                    if let Some(model) = guard.response_model.as_deref() {
                        guard.labels.response_model(model);
                    }
                    guard.finish("success");
                    yield Ok(ChatStreamEvent::Finished);
                    return;
                }
                Err(error) => {
                    guard.finish("stream_error");
                    yield Err(error);
                    return;
                }
            }
        }
    })
}

#[cfg(test)]
mod response_model_tests {
    use super::*;

    async fn observed(events: Vec<ChatStreamEvent>) -> Vec<anyhow::Result<ChatStreamEvent>> {
        let permit = Arc::new(Semaphore::new(1)).acquire_owned().await.unwrap();
        observe_stream(
            Box::pin(futures::stream::iter(events.into_iter().map(Ok))),
            RequestLabels::new(
                "deepseek",
                "deepseek-v4-flash",
                "test-key",
                LlmOperation::CharacterChat,
                "stream",
                1_024,
            ),
            Instant::now(),
            permit,
            TokioInstant::now() + Duration::from_secs(1),
        )
        .collect()
        .await
    }

    #[test]
    fn stream_accepts_one_observed_model_and_rejects_model_drift() {
        tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap()
            .block_on(async {
                assert!(matches!(
                    observed(vec![
                        ChatStreamEvent::ResponseModel("deepseek-v4-flash".into()),
                        ChatStreamEvent::Finished,
                    ])
                    .await
                    .as_slice(),
                    [Ok(ChatStreamEvent::Finished)]
                ));

                let drift = observed(vec![
                    ChatStreamEvent::ResponseModel("deepseek-v4-flash".into()),
                    ChatStreamEvent::ResponseModel("other-model".into()),
                    ChatStreamEvent::Finished,
                ])
                .await;
                assert_eq!(
                    drift[0].as_ref().unwrap_err().to_string(),
                    "LLM response model changed during the stream"
                );
            });
    }
}
