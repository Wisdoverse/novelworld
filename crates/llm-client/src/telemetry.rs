use std::{
    fmt::Write,
    num::NonZeroU32,
    time::{Duration, Instant},
};

use metrics::{counter, gauge, histogram};
use metrics_exporter_prometheus::PrometheusBuilder;
use sha2::{Digest, Sha256};

pub use metrics_exporter_prometheus::PrometheusHandle as MetricsHandle;

use crate::{LlmOperation, Usage};

const MAX_LABEL_CHARS: usize = 200;
const USAGE_KEY_FINGERPRINT_DOMAIN: &[u8] = b"novelworld-llm-usage-v1\0";
const LLM_SUMMARY_BUCKET_DURATION: Duration = Duration::from_secs(30 * 60);
const LLM_SUMMARY_BUCKET_COUNT: NonZeroU32 = NonZeroU32::new(3).expect("non-zero bucket count");
const LLM_QUANTILES: &[f64] = &[0.0, 0.5, 0.9, 0.95, 0.99, 0.999, 1.0];

pub fn usage_key_fingerprint(api_key: &str) -> String {
    let mut digest = Sha256::new();
    digest.update(USAGE_KEY_FINGERPRINT_DOMAIN);
    digest.update(api_key.as_bytes());
    digest
        .finalize()
        .iter()
        .fold(String::with_capacity(64), |mut fingerprint, byte| {
            write!(fingerprint, "{byte:02x}").expect("writing to a String cannot fail");
            fingerprint
        })
}

pub fn install_metrics(service: &'static str) -> anyhow::Result<MetricsHandle> {
    let handle = PrometheusBuilder::new()
        // Three 30-minute buckets retain every sample for at least 60 minutes,
        // covering the 45-minute release journey without changing the schema.
        .set_quantiles(LLM_QUANTILES)?
        .set_bucket_duration(LLM_SUMMARY_BUCKET_DURATION)?
        .set_bucket_count(LLM_SUMMARY_BUCKET_COUNT)
        .add_global_label("service", service)
        .add_global_label("contract", "llm-observability-v1")
        .install_recorder()?;
    gauge!("novelworld_llm_observability_info").set(1.0);
    for operation in LlmOperation::ALL {
        gauge!(
            "novelworld_llm_operation_output_token_ceiling",
            "operation" => operation.to_str(),
        )
        .set(operation.max_output_tokens() as f64);
    }
    Ok(handle)
}

#[derive(Clone)]
pub(crate) struct RequestLabels {
    provider: String,
    model: String,
    usage_key: String,
    operation: &'static str,
    mode: &'static str,
    output_token_limit: u32,
}

impl RequestLabels {
    pub(crate) fn new(
        provider: &str,
        model: &str,
        api_key: &str,
        operation: LlmOperation,
        mode: &'static str,
        output_token_limit: u32,
    ) -> Self {
        Self {
            provider: bounded_label(provider),
            model: bounded_label(model),
            usage_key: usage_key_fingerprint(api_key),
            operation: operation.to_str(),
            mode,
            output_token_limit,
        }
    }

    pub(crate) fn started(&self) {
        counter!(
            "novelworld_llm_requests_started_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "mode" => self.mode,
        )
        .increment(1);
        histogram!(
            "novelworld_llm_output_token_limit",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "mode" => self.mode,
        )
        .record(self.output_token_limit as f64);
    }

    pub(crate) fn attempt(&self, status: &'static str, elapsed: f64) {
        counter!(
            "novelworld_llm_attempts_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "mode" => self.mode,
            "status" => status,
        )
        .increment(1);
        histogram!(
            "novelworld_llm_attempt_duration_seconds",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "mode" => self.mode,
            "status" => status,
        )
        .record(elapsed);
    }

    pub(crate) fn retry(&self, reason: &'static str) {
        counter!(
            "novelworld_llm_retries_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "mode" => self.mode,
            "reason" => reason,
        )
        .increment(1);
    }

    pub(crate) fn stream_setup(&self, status: &'static str, elapsed: f64) {
        histogram!(
            "novelworld_llm_stream_setup_duration_seconds",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "status" => status,
        )
        .record(elapsed);
    }

    pub(crate) fn response_model(&self, response_model: &str) {
        tracing::info!(
            provider = %self.provider,
            configured_model = %self.model,
            response_model = %bounded_label(response_model),
            operation = self.operation,
            mode = self.mode,
            "LLM response model observed"
        );
    }

    pub(crate) fn finish(&self, status: &'static str, started: Instant) {
        counter!(
            "novelworld_llm_requests_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "mode" => self.mode,
            "status" => status,
        )
        .increment(1);
        histogram!(
            "novelworld_llm_request_duration_seconds",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "mode" => self.mode,
            "status" => status,
        )
        .record(started.elapsed().as_secs_f64());
    }

    pub(crate) fn first_token(&self, started: Instant) {
        histogram!(
            "novelworld_llm_first_token_duration_seconds",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
        )
        .record(started.elapsed().as_secs_f64());
    }

    pub(crate) fn usage(&self, usage: Option<&Usage>) {
        let Some(usage) = usage else {
            counter!(
                "novelworld_llm_usage_reports_total",
                "provider" => self.provider.clone(),
                "model" => self.model.clone(),
                "operation" => self.operation,
                "mode" => self.mode,
                "status" => "missing",
            )
            .increment(1);
            return;
        };

        counter!(
            "novelworld_llm_usage_reports_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "mode" => self.mode,
            "status" => "present",
        )
        .increment(1);
        self.record_usage(usage);
    }

    pub(crate) fn additional_usage(&self, usage: &Usage) {
        self.record_usage(usage);
    }

    fn record_usage(&self, usage: &Usage) {
        self.tokens("input", usage.input_tokens);
        self.tokens("output", usage.output_tokens);
        self.tokens_per_request("input", usage.input_tokens);
        self.tokens_per_request("output", usage.output_tokens);
        if let Some(cached) = usage.cached_input_tokens {
            self.tokens("cached_input", cached);
            self.tokens_per_request("cached_input", cached);
            self.billable_tokens("cached_input", cached);
            self.billable_tokens("uncached_input", usage.input_tokens - cached);
        } else {
            self.billable_tokens("uncached_input", usage.input_tokens);
        }
        self.billable_tokens("output", usage.output_tokens);
    }

    fn tokens(&self, token_type: &'static str, value: u32) {
        counter!(
            "novelworld_llm_tokens_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "type" => token_type,
        )
        .increment(value.into());
    }

    fn tokens_per_request(&self, token_type: &'static str, value: u32) {
        histogram!(
            "novelworld_llm_tokens_per_request",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "type" => token_type,
        )
        .record(value as f64);
    }

    fn billable_tokens(&self, class: &'static str, value: u32) {
        counter!(
            "novelworld_llm_billable_tokens_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "operation" => self.operation,
            "class" => class,
            "usage_key" => self.usage_key.clone(),
        )
        .increment(value.into());
    }
}

/// Embeddings do not yet have the token-usage semantics required by the
/// closed `llm-observability-v1` budget contract. Keep their bounded transport
/// telemetry in a separate namespace so release qualification cannot mistake
/// them for a chat operation.
#[derive(Clone)]
pub(crate) struct EmbeddingLabels {
    provider: String,
    model: String,
}

impl EmbeddingLabels {
    pub(crate) fn new(provider: &str, model: &str) -> Self {
        Self {
            provider: bounded_label(provider),
            model: bounded_label(model),
        }
    }

    pub(crate) fn started(&self) {
        counter!(
            "novelworld_embedding_requests_started_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
        )
        .increment(1);
    }

    pub(crate) fn attempt(&self, status: &'static str, elapsed: f64) {
        counter!(
            "novelworld_embedding_attempts_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "status" => status,
        )
        .increment(1);
        histogram!(
            "novelworld_embedding_attempt_duration_seconds",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "status" => status,
        )
        .record(elapsed);
    }

    pub(crate) fn retry(&self, reason: &'static str) {
        counter!(
            "novelworld_embedding_retries_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "reason" => reason,
        )
        .increment(1);
    }

    pub(crate) fn finish(&self, status: &'static str, started: Instant) {
        counter!(
            "novelworld_embedding_requests_total",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "status" => status,
        )
        .increment(1);
        histogram!(
            "novelworld_embedding_request_duration_seconds",
            "provider" => self.provider.clone(),
            "model" => self.model.clone(),
            "status" => status,
        )
        .record(started.elapsed().as_secs_f64());
    }
}

fn bounded_label(value: &str) -> String {
    if value.is_empty()
        || value.chars().count() > MAX_LABEL_CHARS
        || value.chars().any(|character| {
            character.is_control()
                || !(character.is_ascii_alphanumeric()
                    || matches!(character, '.' | '-' | '_' | '/' | ':'))
        })
    {
        "invalid".into()
    } else {
        value.into()
    }
}

#[cfg(test)]
mod tests {
    use super::{bounded_label, usage_key_fingerprint};

    #[test]
    fn labels_are_bounded_and_cannot_carry_secrets_or_control_text() {
        assert_eq!(bounded_label("deepseek-v4-pro"), "deepseek-v4-pro");
        assert_eq!(bounded_label("https://secret.invalid?q=key"), "invalid");
        assert_eq!(bounded_label("line\nbreak"), "invalid");
        assert_eq!(bounded_label(&"x".repeat(201)), "invalid");
    }

    #[test]
    fn usage_key_fingerprint_is_stable_bounded_and_one_way() {
        let fingerprint = usage_key_fingerprint("sk-secret");
        assert_eq!(fingerprint.len(), 64);
        assert!(fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(fingerprint, usage_key_fingerprint("sk-secret"));
        assert_ne!(fingerprint, usage_key_fingerprint("sk-other"));
        assert!(!fingerprint.contains("secret"));
    }
}
