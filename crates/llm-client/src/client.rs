use anyhow::{anyhow, Result};
use async_stream::stream;
use futures::StreamExt;
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::Instant as TokioInstant,
};

use crate::providers::openai::OpenAIProvider;
use crate::retry::RetryPolicy;
use crate::telemetry::{EmbeddingLabels, RequestLabels};
use crate::types::*;

pub struct LlmClient {
    http: reqwest::Client,
    provider: Option<ConfiguredProvider>,
    admission: Arc<Semaphore>,
}

struct ConfiguredProvider {
    name: String,
    api_key: String,
    transport: OpenAIProvider,
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
            provider: None,
            admission: shared_admission(),
        }
    }

    fn admit(&self) -> Result<OwnedSemaphorePermit> {
        self.admission
            .clone()
            .try_acquire_owned()
            .map_err(|_| anyhow!("LLM request capacity is busy"))
    }

    pub fn with_openai_compatible(
        mut self,
        name: impl Into<String>,
        api_key: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        let url = base_url.into();
        self.provider = Some(ConfiguredProvider {
            name: name.into(),
            api_key: api_key.into(),
            transport: OpenAIProvider::new(Some(&url)),
        });
        self
    }

    fn resolve_provider(&self, model: &str) -> Result<(&OpenAIProvider, &str, String, String)> {
        let provider = self
            .provider
            .as_ref()
            .ok_or_else(|| anyhow!("LLM transport is not configured"))?;
        let model_name = match model.split_once('/') {
            Some((name, model_name)) if name == provider.name => model_name,
            Some((name, _)) => {
                return Err(anyhow!(
                    "LLM request names provider '{name}', but '{}' is configured",
                    provider.name
                ))
            }
            None => model,
        };
        if model_name.is_empty() {
            return Err(anyhow!("LLM request model is empty"));
        }
        Ok((
            &provider.transport,
            &provider.api_key,
            provider.name.clone(),
            model_name.to_owned(),
        ))
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
        let started = Instant::now();
        let (provider, api_key, provider_name, model_name) =
            self.resolve_provider(&request.model)?;
        let _permit = self.admit()?;
        let labels = EmbeddingLabels::new(&provider_name, &model_name);
        labels.started();
        let req = EmbeddingRequest {
            model: model_name,
            input: request.input,
        };
        let deadline = TokioInstant::now() + LLM_TOTAL_TIMEOUT;
        match tokio::time::timeout_at(deadline, async {
            for attempt in 0..=RetryPolicy::max_retries() {
                let attempt_started = Instant::now();
                match provider.embed(&self.http, api_key, &req).await {
                    Ok(response) => {
                        labels.attempt("success", attempt_started.elapsed().as_secs_f64());
                        labels.finish("success", started);
                        return Ok(response);
                    }
                    Err(error) => {
                        let api_error = error.downcast_ref::<LlmApiError>();
                        let status = api_error.map(|error| error.status).unwrap_or(500);
                        let metric_status = error_status(api_error);
                        labels.attempt(metric_status, attempt_started.elapsed().as_secs_f64());

                        if RetryPolicy::should_retry(status, attempt) {
                            labels.retry(metric_status);
                            let delay = RetryPolicy::delay(
                                status,
                                attempt,
                                api_error.and_then(|error| error.retry_after.as_deref()),
                            );
                            tracing::warn!(
                                "LLM embedding error ({}), retry {}/{}: {}",
                                status,
                                attempt + 1,
                                RetryPolicy::max_retries(),
                                error
                            );
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        labels.finish("error", started);
                        return Err(error);
                    }
                }
            }
            unreachable!()
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
