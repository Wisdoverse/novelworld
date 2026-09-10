use std::{sync::Arc, time::Duration};

use crate::diagnostic_budget::{Binding, BudgetClient, BudgetControlError};
use anyhow::{anyhow, Result};
use futures::StreamExt;
use serde::Deserialize;
use tokio::sync::OnceCell;
use tokio::time::Instant;

use crate::{ChatRequest, ChatResponse, ChatStream, LlmClient};

#[derive(Debug)]
pub struct NotConfigured;

impl std::fmt::Display for NotConfigured {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("runtime LLM configuration is not configured")
    }
}

impl std::error::Error for NotConfigured {}

pub struct RuntimeLlmClient {
    budget: std::result::Result<Option<Arc<BudgetClient>>, BudgetControlError>,
    source: ConfigSource,
    resolved: OnceCell<Arc<ResolvedClient>>,
}

enum ConfigSource {
    Static(RuntimeConfig),
    Remote {
        client: reqwest::Client,
        user_service_url: String,
        token: String,
        allow_insecure_http: bool,
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
    diagnostic_budget: Option<Binding>,
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
        crate::validate_internal_service_token(&token)?;
        let allow_insecure_http = std::env::var("LLM_ALLOW_INSECURE_HTTP")
            .ok()
            .is_some_and(|value| value.eq_ignore_ascii_case("true"));
        let instance = Self::remote_with_http_policy(user_service_url, token, allow_insecure_http);
        instance.budget.as_ref().map_err(|error| *error)?;
        Ok(instance)
    }

    pub fn static_config(
        api_url: String,
        model: String,
        api_key: String,
        thinking_enabled: bool,
    ) -> Self {
        Self {
            budget: BudgetClient::from_environment(),
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

    fn remote_with_http_policy(
        user_service_url: String,
        token: String,
        allow_insecure_http: bool,
    ) -> Self {
        let budget = BudgetClient::from_environment();
        let client = if matches!(&budget, Ok(Some(_))) {
            reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .build()
                .expect("valid static runtime config HTTP policy")
        } else {
            reqwest::Client::new()
        };
        Self {
            budget,
            source: ConfigSource::Remote {
                client,
                user_service_url: user_service_url.trim_end_matches('/').into(),
                token,
                allow_insecure_http,
            },
            resolved: OnceCell::new(),
        }
    }

    async fn resolved(&self, runtime_user_id: Option<&str>) -> Result<Arc<ResolvedClient>> {
        let budget = self.budget.as_ref().map_err(|error| *error)?;
        if let ConfigSource::Remote {
            client,
            user_service_url,
            token,
            allow_insecure_http,
        } = &self.source
        {
            let response = remote_config_request(client, user_service_url, token, runtime_user_id)
                .send()
                .await?;
            validate_remote_status(response.status())?;
            let config = if budget.is_some() {
                let mut body = Vec::new();
                let mut stream = response.bytes_stream();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| BudgetControlError)?;
                    // Config includes the bounded API key; it is not the credential-free 4 KiB budget API.
                    if body.len().saturating_add(chunk.len()) > 8192 {
                        return Err(BudgetControlError.into());
                    }
                    body.extend_from_slice(&chunk);
                }
                serde_json::from_slice(&body).map_err(|_| BudgetControlError)?
            } else {
                response.json().await?
            };
            return Ok(Arc::new(build_resolved(
                validate_remote_config(
                    config,
                    *allow_insecure_http,
                    budget.as_ref().map(|budget| budget.binding()),
                )?,
                budget.clone(),
            )));
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
                Ok(Arc::new(build_resolved(config, budget.clone())))
            })
            .await
            .cloned()
    }

    pub async fn chat(&self, mut request: ChatRequest) -> Result<ChatResponse> {
        let deadline = self
            .budget
            .as_ref()
            .map_err(|error| *error)?
            .as_ref()
            .map(|_| Instant::now() + Duration::from_secs(300));
        let resolved = self
            .resolved_with_deadline(request.runtime_user_id.as_deref(), deadline)
            .await?;
        request.model.clone_from(&resolved.model);
        if request.thinking.is_none() {
            request.thinking = Some(resolved.thinking_enabled);
        }
        resolved.client.chat_with_deadline(request, deadline).await
    }

    pub async fn chat_stream(&self, mut request: ChatRequest) -> Result<ChatStream> {
        let deadline = self
            .budget
            .as_ref()
            .map_err(|error| *error)?
            .as_ref()
            .map(|_| Instant::now() + Duration::from_secs(300));
        let resolved = self
            .resolved_with_deadline(request.runtime_user_id.as_deref(), deadline)
            .await?;
        request.model.clone_from(&resolved.model);
        if request.thinking.is_none() {
            request.thinking = Some(resolved.thinking_enabled);
        }
        resolved
            .client
            .chat_stream_with_deadline(request, deadline)
            .await
    }

    async fn resolved_with_deadline(
        &self,
        user_id: Option<&str>,
        deadline: Option<Instant>,
    ) -> Result<Arc<ResolvedClient>> {
        match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, self.resolved(user_id))
                .await
                .map_err(|_| BudgetControlError)?,
            None => self.resolved(user_id).await,
        }
    }

    /// Generate prose where the output itself is the product. Reasoning mode
    /// is deliberately disabled so providers such as DeepSeek cannot consume
    /// the response budget with hidden reasoning and return an incomplete
    /// chapter instead of usable text.
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

    pub async fn json_chat_for_user(
        &self,
        runtime_user_id: impl Into<String>,
        operation: crate::LlmOperation,
        prompt: &str,
    ) -> Result<String> {
        self.chat(production_json_request(operation, prompt).runtime_user_id(runtime_user_id))
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

/// Build the JSON request used by production domain adapters.
///
/// Qualification tools reuse this narrow constructor so they measure the
/// deployed request contract instead of a hand-maintained approximation.
pub fn production_json_request(operation: crate::LlmOperation, prompt: &str) -> ChatRequest {
    let temperature = if matches!(
        operation,
        crate::LlmOperation::CharacterExtraction | crate::LlmOperation::CanonExtraction
    ) {
        0.0
    } else {
        0.3
    };
    ChatRequest::new(operation, "")
        .message(
            "system",
            "You are a helpful assistant that always responds with a non-empty valid JSON object. Output JSON only.",
        )
        .message("user", prompt)
        .temperature(temperature)
        .max_tokens(operation.max_output_tokens())
        .thinking(false)
        .json()
}

fn build_resolved(config: RuntimeConfig, budget: Option<Arc<BudgetClient>>) -> ResolvedClient {
    let model = format!("{}/{}", config.provider, config.model);
    let mut client =
        LlmClient::new().with_openai_compatible(&config.provider, config.api_key, config.api_url);
    client.budget = Ok(budget);
    ResolvedClient {
        client,
        model,
        thinking_enabled: config.thinking_enabled,
    }
}

fn validate_remote_config(
    config: RemoteConfig,
    allow_insecure_http: bool,
    binding: Option<&Binding>,
) -> Result<RuntimeConfig> {
    let contract_valid = match binding {
        None => config.contract == 2 && config.diagnostic_budget.is_none(),
        Some(expected) => {
            config.contract == 3 && config.diagnostic_budget.as_ref() == Some(expected)
        }
    };
    if binding.is_some() {
        let profile = crate::diagnostic_budget::profile();
        let origin = crate::diagnostic_budget::root_url(&config.api_url)?;
        if !contract_valid
            || origin.origin().ascii_serialization() != profile.origin
            || config.model != profile.model
            || config.thinking_enabled
        {
            return Err(BudgetControlError.into());
        }
    }
    let transport_allowed = reqwest::Url::parse(&config.api_url)
        .ok()
        .is_some_and(|url| {
            url.scheme() == "https" || (allow_insecure_http && url.scheme() == "http")
        });
    if !contract_valid
        || !transport_allowed
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

fn validate_remote_status(status: reqwest::StatusCode) -> Result<()> {
    if status == reqwest::StatusCode::CONFLICT {
        return Err(NotConfigured.into());
    }
    if !status.is_success() {
        return Err(anyhow!(
            "runtime LLM configuration is unavailable ({status})"
        ));
    }
    Ok(())
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

    fn diagnostic_binding() -> Binding {
        Binding::new(uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap())
    }

    fn ordinary_config(api_url: &str) -> RemoteConfig {
        RemoteConfig {
            contract: 2,
            diagnostic_budget: None,
            api_url: api_url.into(),
            model: "ordinary-model".into(),
            api_key: "secret".into(),
            thinking_enabled: false,
        }
    }

    fn budget_config(binding: &Binding) -> RemoteConfig {
        let profile = crate::diagnostic_budget::profile();
        RemoteConfig {
            contract: 3,
            diagnostic_budget: Some(binding.clone()),
            api_url: profile.origin.clone(),
            model: profile.model.clone(),
            api_key: "secret".into(),
            thinking_enabled: false,
        }
    }

    #[test]
    fn remote_configuration_transport_is_fail_closed_by_default() {
        let config = |api_url: &str| RemoteConfig {
            contract: 2,
            diagnostic_budget: None,
            api_url: api_url.into(),
            model: "model".into(),
            api_key: "secret".into(),
            thinking_enabled: false,
        };

        assert!(validate_remote_config(config("http://llm-stub:18080"), false, None).is_err());
        assert!(validate_remote_config(config("http://llm-stub:18080"), true, None).is_ok());
        assert!(validate_remote_config(config("https://api.example.com"), false, None).is_ok());
    }

    #[test]
    fn remote_configuration_contracts_are_bound_to_mode() {
        let binding = diagnostic_binding();
        assert!(
            validate_remote_config(ordinary_config("https://api.example.com"), false, None).is_ok()
        );
        assert!(validate_remote_config(budget_config(&binding), false, Some(&binding)).is_ok());

        assert!(validate_remote_config(budget_config(&binding), false, None).is_err());
        assert!(validate_remote_config(
            ordinary_config("https://api.example.com"),
            false,
            Some(&binding)
        )
        .is_err());
    }

    #[test]
    fn budget_remote_configuration_rejects_binding_and_profile_mismatches() {
        let binding = diagnostic_binding();
        let other_binding =
            Binding::new(uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440001").unwrap());

        let wrong_budget_id = budget_config(&other_binding);
        assert!(validate_remote_config(wrong_budget_id, false, Some(&binding)).is_err());

        let mut wrong_contract = budget_config(&binding);
        wrong_contract.contract = 2;
        assert!(validate_remote_config(wrong_contract, false, Some(&binding)).is_err());

        let mut wrong_profile = budget_config(&binding);
        wrong_profile.diagnostic_budget.as_mut().unwrap().profile = "other-profile".into();
        assert!(validate_remote_config(wrong_profile, false, Some(&binding)).is_err());

        let mut wrong_digest = budget_config(&binding);
        wrong_digest
            .diagnostic_budget
            .as_mut()
            .unwrap()
            .profile_sha256 =
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into();
        assert!(validate_remote_config(wrong_digest, false, Some(&binding)).is_err());
    }

    #[test]
    fn budget_remote_configuration_remains_strict_with_insecure_http_enabled() {
        let binding = diagnostic_binding();
        for (api_url, model, thinking_enabled) in [
            ("http://api.deepseek.com/", "deepseek-flash", false),
            ("https://api.deepseek.com/v1", "deepseek-flash", false),
            ("https://api.deepseek.com/", "other-model", false),
            ("https://api.deepseek.com/", "deepseek-flash", true),
        ] {
            let mut config = budget_config(&binding);
            config.api_url = api_url.into();
            config.model = model.into();
            config.thinking_enabled = thinking_enabled;
            assert!(validate_remote_config(config, true, Some(&binding)).is_err());
        }
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

    #[test]
    fn production_json_request_contract_is_fixed() {
        for operation in [
            crate::LlmOperation::CharacterExtraction,
            crate::LlmOperation::CanonExtraction,
        ] {
            let request = production_json_request(operation, "prompt");
            assert_eq!(request.operation, operation);
            assert_eq!(request.runtime_user_id, None);
            assert!(request.model.is_empty());
            assert_eq!(request.messages.len(), 2);
            assert_eq!(request.messages[0].role, "system");
            assert_eq!(request.messages[1].role, "user");
            assert_eq!(request.messages[1].content, "prompt");
            assert_eq!(request.temperature, Some(0.0));
            assert_eq!(request.max_tokens, Some(operation.max_output_tokens()));
            assert!(!request.stream);
            assert!(request.json_mode);
            assert_eq!(request.thinking, Some(false));
        }
        assert_eq!(
            production_json_request(crate::LlmOperation::NarrativeNodeDetection, "prompt")
                .temperature,
            Some(0.3)
        );
    }

    #[test]
    fn not_configured_remains_downcastable_through_anyhow() {
        let error = validate_remote_status(reqwest::StatusCode::CONFLICT).unwrap_err();
        assert!(error.downcast_ref::<NotConfigured>().is_some());
        assert!(validate_remote_status(reqwest::StatusCode::BAD_GATEWAY)
            .unwrap_err()
            .downcast_ref::<NotConfigured>()
            .is_none());
    }
}
