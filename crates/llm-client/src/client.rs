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

use crate::diagnostic_budget::{BudgetClient, BudgetControlError, BudgetEvidenceError, Grant};
use crate::providers::openai::OpenAIProvider;
use crate::retry::RetryPolicy;
use crate::telemetry::{EmbeddingLabels, RequestLabels};
use crate::types::*;

pub struct LlmClient {
    pub(crate) budget: std::result::Result<Option<Arc<BudgetClient>>, BudgetControlError>,
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
    #[cfg(test)]
    pub(crate) fn diagnostic_test_client(budget: Arc<BudgetClient>, dispatch_base: String) -> Self {
        let mut client = Self::new().with_openai_compatible(
            "deepseek",
            "synthetic-test-key",
            "https://api.deepseek.com",
        );
        client.budget = Ok(Some(budget));
        client.admission = Arc::new(Semaphore::new(8));
        client
            .provider
            .as_mut()
            .unwrap()
            .transport
            .test_dispatch_base = Some(dispatch_base);
        client
    }
    pub fn new() -> Self {
        Self {
            budget: BudgetClient::from_environment(),
            http: reqwest::Client::builder()
                .connect_timeout(LLM_CONNECT_TIMEOUT)
                .timeout(LLM_TOTAL_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
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
        self.chat_with_deadline(request, None).await
    }

    pub(crate) async fn chat_with_deadline(
        &self,
        request: ChatRequest,
        deadline: Option<TokioInstant>,
    ) -> Result<ChatResponse> {
        let budget = self.budget.as_ref().map_err(|error| *error)?.clone();
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
        req.stream = false;

        let deadline = deadline.unwrap_or_else(|| TokioInstant::now() + LLM_TOTAL_TIMEOUT);
        let mut provider_started = false;
        let mut pending_attempt = None;
        match tokio::time::timeout_at(deadline, async {
            let mut retry_attempt = 0;
            let mut missing_attempt_usage = false;
            loop {
                provider_started = false;
                let grant = match reserve_attempt(
                    budget.as_ref(),
                    provider,
                    &provider_name,
                    &req,
                    deadline,
                )
                .await
                {
                    Ok(grant) => grant,
                    Err(error) => {
                        labels.finish("budget_error", started);
                        return Err(error);
                    }
                };
                let attempt_started = Instant::now();
                provider_started = true;
                pending_attempt = Some(attempt_started);
                let response = provider.chat(&self.http, api_key, &req).await;
                pending_attempt = None;
                match response {
                    Ok(resp) => {
                        // Provider work occurred even if its subsequent ledger ACK is lost.
                        labels.attempt("success", attempt_started.elapsed().as_secs_f64());
                        if provider.reports_response_model() {
                            labels.response_model(&resp.model);
                        }
                        if missing_attempt_usage {
                            if let Some(usage) = &resp.usage {
                                labels.additional_usage(usage);
                            }
                            labels.usage(None);
                        } else {
                            labels.usage(resp.usage.as_ref());
                        }
                        if let (Some(budget), Some(grant)) = (&budget, grant) {
                            if let Err(error) = budget
                                .settle(grant, Some(&resp.model), resp.usage.as_ref(), deadline)
                                .await
                            {
                                labels.finish("evidence_error", started);
                                return Err(error.into());
                            }
                        }
                        labels.finish("success", started);
                        return Ok(resp);
                    }
                    Err(e) => {
                        if budget.is_some() && e.is::<crate::providers::openai::InvalidCompletion>()
                        {
                            labels
                                .attempt("evidence_error", attempt_started.elapsed().as_secs_f64());
                            labels.finish("evidence_error", started);
                            return Err(BudgetEvidenceError.into());
                        }
                        if e.is::<ResponseEvidenceError>() {
                            labels
                                .attempt("evidence_error", attempt_started.elapsed().as_secs_f64());
                            labels.finish("evidence_error", started);
                            return Err(e);
                        }
                        if req.json_mode && e.downcast_ref::<JsonModeEmpty>().is_some() {
                            let empty = e.downcast_ref::<JsonModeEmpty>().unwrap();
                            labels.attempt(
                                "empty_json_mode",
                                attempt_started.elapsed().as_secs_f64(),
                            );
                            labels.response_model(&empty.model);
                            if let Some(usage) = &empty.usage {
                                labels.additional_usage(usage);
                            } else {
                                missing_attempt_usage = true;
                            }
                            if let (Some(budget), Some(grant)) = (&budget, grant) {
                                if !empty.complete_empty {
                                    labels.finish("evidence_error", started);
                                    return Err(BudgetEvidenceError.into());
                                }
                                if let Err(error) = budget
                                    .settle(
                                        grant,
                                        Some(&empty.model),
                                        empty.usage.as_ref(),
                                        deadline,
                                    )
                                    .await
                                {
                                    labels.finish("evidence_error", started);
                                    return Err(error.into());
                                }
                            }
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
                if let Some(attempt_started) = pending_attempt {
                    labels.attempt("timeout", attempt_started.elapsed().as_secs_f64());
                }
                labels.finish("timeout", started);
                if budget.is_some() {
                    if provider_started {
                        Err(BudgetEvidenceError.into())
                    } else {
                        Err(BudgetControlError.into())
                    }
                } else if req.response_observer.is_some() {
                    Err(ResponseEvidenceError.into())
                } else {
                    Err(anyhow!("LLM request exceeded the total deadline"))
                }
            }
        }
    }

    pub async fn chat_stream(&self, request: ChatRequest) -> Result<ChatStream> {
        self.chat_stream_with_deadline(request, None).await
    }

    pub(crate) async fn chat_stream_with_deadline(
        &self,
        request: ChatRequest,
        deadline: Option<TokioInstant>,
    ) -> Result<ChatStream> {
        let budget = self.budget.as_ref().map_err(|error| *error)?.clone();
        if request.response_observer.is_some() {
            return Err(anyhow!(
                "response evidence observers require non-streaming chat"
            ));
        }
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

        let deadline = deadline.unwrap_or_else(|| TokioInstant::now() + LLM_TOTAL_TIMEOUT);
        let mut provider_started = false;
        let mut pending_attempt = None;
        let (upstream, grant) = match tokio::time::timeout_at(deadline, async {
            for attempt in 0..=RetryPolicy::max_retries() {
                provider_started = false;
                let grant = match reserve_attempt(
                    budget.as_ref(),
                    provider,
                    &provider_name,
                    &req,
                    deadline,
                )
                .await
                {
                    Ok(grant) => grant,
                    Err(error) => {
                        labels.finish("budget_error", started);
                        return Err(error);
                    }
                };
                let attempt_started = Instant::now();
                provider_started = true;
                pending_attempt = Some(attempt_started);
                let response = provider.chat_stream(&self.http, api_key, &req).await;
                pending_attempt = None;
                match response {
                    Ok(upstream) => {
                        let setup = attempt_started.elapsed().as_secs_f64();
                        labels.attempt("success", setup);
                        labels.stream_setup("success", setup);
                        return Ok((upstream, grant));
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
                if let Some(attempt_started) = pending_attempt {
                    let elapsed = attempt_started.elapsed().as_secs_f64();
                    labels.attempt("timeout", elapsed);
                    labels.stream_setup("timeout", elapsed);
                }
                labels.finish("setup_timeout", started);
                if budget.is_some() {
                    return if provider_started {
                        Err(BudgetEvidenceError.into())
                    } else {
                        Err(BudgetControlError.into())
                    };
                }
                return Err(anyhow!("LLM request exceeded the total deadline"));
            }
        };
        Ok(observe_stream(
            upstream,
            labels,
            started,
            permit,
            deadline,
            budget.zip(grant),
        ))
    }

    pub async fn embed(&self, request: EmbeddingRequest) -> Result<EmbeddingResponse> {
        if self.budget.as_ref().map_err(|error| *error)?.is_some() {
            return Err(BudgetControlError.into());
        }
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

async fn reserve_attempt(
    budget: Option<&Arc<BudgetClient>>,
    provider: &OpenAIProvider,
    provider_name: &str,
    request: &ChatRequest,
    deadline: TokioInstant,
) -> Result<Option<Grant>> {
    let Some(budget) = budget else {
        return Ok(None);
    };
    let wire = provider
        .chat_wire_bytes(request)
        .map_err(|_| BudgetControlError)?;
    Ok(Some(
        budget
            .reserve(provider_name, provider.base_url(), request, &wire, deadline)
            .await?,
    ))
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
    mut settlement: Option<(Arc<BudgetClient>, Grant)>,
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
        let budgeted = settlement.is_some();
        let evidence_error = |error: anyhow::Error| -> anyhow::Error {
            if budgeted { BudgetEvidenceError.into() } else { error }
        };
        let failure_status = if budgeted { "evidence_error" } else { "stream_error" };
        loop {
            let item = match tokio::time::timeout_at(deadline, upstream.next()).await {
                Ok(item) => item,
                Err(_) => {
                    guard.finish("timeout");
                    yield Err(evidence_error(anyhow!("LLM request exceeded the total deadline")));
                    return;
                }
            };
            let Some(item) = item else {
                guard.finish(failure_status);
                yield Err(evidence_error(anyhow!("LLM stream ended without a terminal event")));
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
                            guard.finish(failure_status);
                            yield Err(evidence_error(anyhow!("LLM response model changed during the stream")));
                            return;
                        }
                    }
                }
                Ok(ChatStreamEvent::Usage(usage)) => {
                    if guard.usage.is_some()
                        || usage.cached_input_tokens.is_some_and(|cached| cached > usage.input_tokens)
                    {
                        guard.finish(failure_status);
                        yield Err(evidence_error(anyhow!("invalid or duplicate LLM stream usage")));
                        return;
                    }
                    guard.usage = Some(usage);
                }
                Ok(ChatStreamEvent::Finished) => {
                    if let Some((budget, grant)) = settlement.take() {
                        if let Err(error) = budget.settle(grant, guard.response_model.as_deref(), guard.usage.as_ref(), deadline).await {
                            guard.finish("evidence_error");
                            yield Err(error.into());
                            return;
                        }
                    }
                    if let Some(model) = guard.response_model.as_deref() {
                        guard.labels.response_model(model);
                    }
                    guard.finish("success");
                    yield Ok(ChatStreamEvent::Finished);
                    return;
                }
                Err(error) => {
                    guard.finish(failure_status);
                    yield Err(evidence_error(error));
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
            None,
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
