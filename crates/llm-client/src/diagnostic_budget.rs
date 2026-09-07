use crate::{ChatRequest, Usage};
use futures::StreamExt;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::time::Instant;
use uuid::{Uuid, Variant, Version};

pub const CONTRACT_HEADER: &str = "X-LLM-Budget-Contract";
pub const MAX_CONTROL_BYTES: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BudgetControlError;

impl std::fmt::Display for BudgetControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("diagnostic_budget_control_failed")
    }
}
impl std::error::Error for BudgetControlError {}

#[derive(Debug)]
pub struct BudgetEvidenceError;

impl std::fmt::Display for BudgetEvidenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("diagnostic_budget_evidence_failed")
    }
}
impl std::error::Error for BudgetEvidenceError {}

/// Canonical spelling is checked before UUID deserialization can erase it.
pub fn canonical_uuid(value: &str) -> Result<Uuid, BudgetControlError> {
    let id = Uuid::parse_str(value).map_err(|_| BudgetControlError)?;
    if id.get_version() != Some(Version::Random)
        || id.get_variant() != Variant::RFC4122
        || id.to_string() != value
    {
        return Err(BudgetControlError);
    }
    Ok(id)
}

fn deserialize_uuid<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Uuid, D::Error> {
    canonical_uuid(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub contract: String,
    pub profile: String,
    pub profile_sha256: String,
    #[serde(deserialize_with = "deserialize_uuid")]
    pub budget_id: Uuid,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Amount {
    pub attempts: u64,
    pub tokens: u64,
    pub cost_micro_cny: u64,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReserveRequest {
    pub binding: Binding,
    #[serde(deserialize_with = "deserialize_uuid")]
    pub attempt_id: Uuid,
    pub provider: String,
    pub model: String,
    pub origin: String,
    pub operation: String,
    pub output_limit: u32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReservationResponse {
    pub binding: Binding,
    #[serde(deserialize_with = "deserialize_uuid")]
    pub attempt_id: Uuid,
    pub ordinal: u64,
    pub reservation: Amount,
}

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SettlementUsage {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettleRequest {
    pub binding: Binding,
    #[serde(deserialize_with = "deserialize_uuid")]
    pub attempt_id: Uuid,
    pub usage: SettlementUsage,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettlementResponse {
    pub binding: Binding,
    #[serde(deserialize_with = "deserialize_uuid")]
    pub attempt_id: Uuid,
    pub usage: SettlementUsage,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SealRequest {
    pub binding: Binding,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SealResponse {
    pub binding: Binding,
    pub sealed: bool,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotResponse {
    pub binding: Binding,
    pub limits: Amount,
    pub charged: Amount,
    pub expires_at: String,
    pub sealed: bool,
}

// This schema reads only trusted compiled JSON. Wire schemas above are separately strict.
#[derive(Deserialize)]
pub(crate) struct CompiledProfile {
    pub contract: String,
    pub profile: String,
    pub provider: String,
    pub model: String,
    pub origin: String,
    pub input_token_ceiling: u64,
    pub input_micro_cny: u64,
    pub output_micro_cny: u64,
    pub max_messages: usize,
    pub max_request_bytes: usize,
    pub max_limits: Amount,
    pub operations: BTreeMap<String, u32>,
}

pub(crate) fn profile() -> &'static CompiledProfile {
    static PROFILE: OnceLock<CompiledProfile> = OnceLock::new();
    PROFILE.get_or_init(|| {
        serde_json::from_str(PROFILE_JSON).expect("valid compiled diagnostic profile")
    })
}

impl Binding {
    pub fn from_environment() -> Result<Option<Self>, BudgetControlError> {
        let id = std::env::var_os("LLM_DIAGNOSTIC_BUDGET_ID")
            .unwrap_or_default()
            .into_string()
            .map_err(|_| BudgetControlError)?;
        let limits = std::env::var_os("LLM_DIAGNOSTIC_BUDGET_LIMITS")
            .unwrap_or_default()
            .into_string()
            .map_err(|_| BudgetControlError)?;
        if id.is_empty() {
            return if limits.is_empty() {
                Ok(None)
            } else {
                Err(BudgetControlError)
            };
        }
        Ok(Some(Self::new(canonical_uuid(&id)?)))
    }

    pub(crate) fn new(budget_id: Uuid) -> Self {
        Self {
            budget_id,
            contract: profile().contract.clone(),
            profile: profile().profile.clone(),
            profile_sha256: profile_sha256(),
        }
    }
}

/// A root URL only. Internal HTTP is permitted; provider origin is checked separately.
pub(crate) fn root_url(value: &str) -> Result<reqwest::Url, BudgetControlError> {
    // URL normalization can erase an empty userinfo component or embedded control bytes.
    if value.bytes().any(|byte| byte.is_ascii_control())
        || value.split_once("://").is_some_and(|(_, rest)| {
            rest.split(['/', '?', '#'])
                .next()
                .is_some_and(|authority| authority.contains('@'))
        })
    {
        return Err(BudgetControlError);
    }
    let url = reqwest::Url::parse(value).map_err(|_| BudgetControlError)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || value.trim() != value
    {
        return Err(BudgetControlError);
    }
    Ok(url)
}

#[derive(Clone)]
pub(crate) struct BudgetClient {
    binding: Binding,
    http: reqwest::Client,
    control_origin: String,
    token: String,
}

/// Deliberately not Clone: one reserve acknowledgement authorizes one reviewed dispatch path.
pub(crate) struct Grant {
    attempt_id: Uuid,
    output_limit: u32,
}

impl BudgetClient {
    pub(crate) fn from_environment() -> Result<Option<Arc<Self>>, BudgetControlError> {
        let Some(binding) = Binding::from_environment()? else {
            return Ok(None);
        };
        let origin = std::env::var("USER_SERVICE_URL").map_err(|_| BudgetControlError)?;
        let token = std::env::var("INTERNAL_SERVICE_TOKEN").map_err(|_| BudgetControlError)?;
        Ok(Some(Arc::new(Self::new(binding, &origin, token)?)))
    }

    pub(crate) fn new(
        binding: Binding,
        origin: &str,
        token: String,
    ) -> Result<Self, BudgetControlError> {
        if binding != Binding::new(canonical_uuid(&binding.budget_id.to_string())?) {
            return Err(BudgetControlError);
        }
        crate::validate_internal_service_token(&token).map_err(|_| BudgetControlError)?;
        let origin = root_url(origin)?;
        Ok(Self {
            binding,
            token,
            control_origin: origin.origin().ascii_serialization(),
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_secs(5))
                .build()
                .map_err(|_| BudgetControlError)?,
        })
    }

    pub(crate) fn binding(&self) -> &Binding {
        &self.binding
    }

    async fn post<B: Serialize, R: DeserializeOwned>(
        &self,
        suffix: &str,
        body: &B,
        deadline: Instant,
    ) -> Result<R, BudgetControlError> {
        let body = serde_json::to_vec(body).map_err(|_| BudgetControlError)?;
        if body.len() > MAX_CONTROL_BYTES || Instant::now() >= deadline {
            return Err(BudgetControlError);
        }
        let deadline = deadline.min(Instant::now() + Duration::from_secs(5));
        tokio::time::timeout_at(deadline, async {
            let response = self
                .http
                .post(format!(
                    "{}/internal/llm-budget/{}/{}",
                    self.control_origin, self.binding.budget_id, suffix
                ))
                .header("X-Internal-Service-Token", &self.token)
                .header(CONTRACT_HEADER, &self.binding.contract)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .await
                .map_err(|_| BudgetControlError)?;
            if response.status() != reqwest::StatusCode::OK
                || response
                    .content_length()
                    .is_some_and(|length| length > MAX_CONTROL_BYTES as u64)
            {
                return Err(BudgetControlError);
            }
            let mut body = Vec::new();
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|_| BudgetControlError)?;
                if body.len().saturating_add(chunk.len()) > MAX_CONTROL_BYTES {
                    return Err(BudgetControlError);
                }
                body.extend_from_slice(&chunk);
            }
            serde_json::from_slice(&body).map_err(|_| BudgetControlError)
        })
        .await
        .map_err(|_| BudgetControlError)?
    }

    pub(crate) async fn reserve(
        &self,
        provider: &str,
        origin: &str,
        request: &ChatRequest,
        wire: &[u8],
        deadline: Instant,
    ) -> Result<Grant, BudgetControlError> {
        let quote = validate_dispatch(provider, origin, request, wire)?;
        let attempt_id = Uuid::new_v4();
        let output_limit = request
            .effective_max_output_tokens()
            .ok_or(BudgetControlError)?;
        let response: ReservationResponse = self
            .post(
                "reserve",
                &ReserveRequest {
                    binding: self.binding.clone(),
                    attempt_id,
                    provider: provider.into(),
                    model: request.model.clone(),
                    origin: profile().origin.clone(),
                    operation: request.operation.to_str().into(),
                    output_limit,
                },
                deadline,
            )
            .await?;
        if response.binding != self.binding
            || response.attempt_id != attempt_id
            || response.ordinal == 0
            || response.ordinal > profile().max_limits.attempts
            || response.reservation != quote
        {
            return Err(BudgetControlError);
        }
        // Lost/late acknowledgements never grant a new deadline; the committed reserve stays charged.
        if Instant::now() >= deadline {
            return Err(BudgetControlError);
        }
        Ok(Grant {
            attempt_id,
            output_limit,
        })
    }

    pub(crate) async fn settle(
        &self,
        grant: Grant,
        model: Option<&str>,
        usage: Option<&Usage>,
        deadline: Instant,
    ) -> Result<(), BudgetEvidenceError> {
        let usage = usage.ok_or(BudgetEvidenceError)?;
        if model != Some(profile().model.as_str())
            || u64::from(usage.input_tokens) > profile().input_token_ceiling
            || usage.output_tokens > grant.output_limit
            || usage
                .cached_input_tokens
                .is_some_and(|cached| cached > usage.input_tokens)
        {
            return Err(BudgetEvidenceError);
        }
        let usage = SettlementUsage {
            model: profile().model.clone(),
            input_tokens: u64::from(usage.input_tokens),
            output_tokens: u64::from(usage.output_tokens),
            cached_input_tokens: usage.cached_input_tokens.map(u64::from),
        };
        let request = SettleRequest {
            binding: self.binding.clone(),
            attempt_id: grant.attempt_id,
            usage,
        };
        let response: SettlementResponse = self
            .post("settle", &request, deadline)
            .await
            .map_err(|_| BudgetEvidenceError)?;
        if response.binding != self.binding
            || response.attempt_id != grant.attempt_id
            || response.usage != request.usage
        {
            return Err(BudgetEvidenceError);
        }
        Ok(())
    }
}

/// Bootstrap validation for the owner, whose Settings tester is constructed only after startup.
pub fn validate_environment() -> Result<(), BudgetControlError> {
    BudgetClient::from_environment().map(|_| ())
}

pub(crate) fn validate_dispatch(
    provider: &str,
    origin: &str,
    request: &ChatRequest,
    wire: &[u8],
) -> Result<Amount, BudgetControlError> {
    let profile = profile();
    let origin = root_url(origin)?;
    let output = request
        .effective_max_output_tokens()
        .ok_or(BudgetControlError)?;
    if origin.origin().ascii_serialization() != profile.origin
        || provider != profile.provider
        || request.model != profile.model
        || request.thinking != Some(false)
        || request.messages.len() > profile.max_messages
        || request
            .messages
            .iter()
            .any(|message| !matches!(message.role.as_str(), "system" | "user" | "assistant"))
        || wire.len() > profile.max_request_bytes
        || output == 0
        || profile
            .operations
            .get(request.operation.to_str())
            .is_none_or(|ceiling| output > *ceiling)
    {
        return Err(BudgetControlError);
    }
    let output = u64::from(output);
    Ok(Amount {
        attempts: 1,
        tokens: profile
            .input_token_ceiling
            .checked_add(output)
            .ok_or(BudgetControlError)?,
        cost_micro_cny: profile
            .input_token_ceiling
            .checked_mul(profile.input_micro_cny)
            .and_then(|input| {
                output
                    .checked_mul(profile.output_micro_cny)
                    .and_then(|output| input.checked_add(output))
            })
            .ok_or(BudgetControlError)?,
    })
}

pub const PROFILE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/llm-budget/diagnostic-v1.json"
));

/// Hash the exact compiled profile bytes; no downloaded policy or mutable runtime pricing.
pub fn profile_sha256() -> String {
    Sha256::digest(PROFILE_JSON.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
