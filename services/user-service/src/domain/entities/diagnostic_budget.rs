use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::{Uuid, Variant, Version};

pub const PROFILE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/llm-budget/diagnostic-v1.json"
));
pub const MEMORY_PROFILE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/llm-budget/diagnostic-v2.json"
));
pub const LOCAL_MEMORY_PROFILE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/llm-budget/diagnostic-v3.json"
));

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BudgetError {
    #[error("diagnostic_budget_invalid")]
    Invalid,
    #[error("diagnostic_budget_exhausted")]
    Exhausted,
    #[error("diagnostic_budget_not_found")]
    NotFound,
    #[error("diagnostic_budget_conflict")]
    Conflict,
    #[error("diagnostic_budget_closed")]
    Closed,
    #[error("diagnostic_budget_unavailable")]
    Unavailable,
}

/// Immutable registration. Normal startup verifies this value; it never provisions it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub budget_id: Uuid,
    pub contract: String,
    pub profile: String,
    pub profile_sha256: String,
    pub limits: Amount,
    pub expires_at: DateTime<Utc>,
}

impl Registration {
    pub fn validate(&self) -> Result<(), BudgetError> {
        let profile = Profile::compiled_named(&self.profile)?;
        if self.budget_id.get_version() != Some(Version::Random)
            || self.budget_id.get_variant() != Variant::RFC4122
            || self.contract != profile.contract
            || self.profile_sha256.len() != 64
            || !self
                .profile_sha256
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            || !self.limits.within(profile.max_limits)
            || self.expires_at.timestamp_subsec_nanos() != 0
        {
            return Err(BudgetError::Invalid);
        }
        Ok(())
    }

    pub fn validate_provision(&self, database_now: DateTime<Utc>) -> Result<(), BudgetError> {
        self.validate()?;
        let remaining = self.expires_at.signed_duration_since(database_now);
        if remaining <= chrono::Duration::zero()
            || remaining
                > chrono::Duration::seconds(
                    Profile::compiled_named(&self.profile)?.max_lifetime_seconds,
                )
        {
            return Err(BudgetError::Invalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BudgetSnapshot {
    pub registration: Registration,
    pub charged: Amount,
    pub sealed: bool,
}

impl BudgetSnapshot {
    pub fn reserve(
        &self,
        quote: Amount,
        database_now: DateTime<Utc>,
    ) -> Result<Amount, BudgetError> {
        if self.sealed || database_now >= self.registration.expires_at {
            return Err(BudgetError::Closed);
        }
        self.charged.reserve(quote, self.registration.limits)
    }
}

#[derive(Clone, Debug)]
pub struct Attempt {
    pub attempt_id: Uuid,
    pub operation: String,
    pub output_limit: u32,
}

pub struct Dispatch {
    pub attempt: Attempt,
    pub provider: String,
    pub model: String,
    pub origin: String,
}

impl Dispatch {
    pub fn validate(&self, profile_name: &str) -> Result<(), BudgetError> {
        let profile = Profile::compiled_named(profile_name)?;
        let (provider, model, origin) = profile.identity(&self.attempt.operation)?;
        if self.provider != provider || self.model != model || self.origin != origin {
            return Err(BudgetError::Invalid);
        }
        self.attempt.quote_with_profile(&profile)?;
        Ok(())
    }
}

impl Attempt {
    pub fn quote(&self) -> Result<Amount, BudgetError> {
        self.quote_with_profile(&Profile::compiled())
    }

    pub fn quote_with_profile(&self, profile: &Profile) -> Result<Amount, BudgetError> {
        if self.attempt_id.get_version() != Some(Version::Random)
            || self.attempt_id.get_variant() != Variant::RFC4122
        {
            return Err(BudgetError::Invalid);
        }
        profile.quote(&self.operation, self.output_limit)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reservation {
    pub attempt_id: Uuid,
    pub ordinal: u64,
    pub amount: Amount,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Amount {
    pub attempts: u64,
    pub tokens: u64,
    pub cost_micro_cny: u64,
}

impl Amount {
    pub fn checked_add(self, other: Self) -> Result<Self, BudgetError> {
        Ok(Self {
            attempts: self
                .attempts
                .checked_add(other.attempts)
                .ok_or(BudgetError::Invalid)?,
            tokens: self
                .tokens
                .checked_add(other.tokens)
                .ok_or(BudgetError::Invalid)?,
            cost_micro_cny: self
                .cost_micro_cny
                .checked_add(other.cost_micro_cny)
                .ok_or(BudgetError::Invalid)?,
        })
    }

    pub fn checked_sub(self, other: Self) -> Result<Self, BudgetError> {
        Ok(Self {
            attempts: self
                .attempts
                .checked_sub(other.attempts)
                .ok_or(BudgetError::Invalid)?,
            tokens: self
                .tokens
                .checked_sub(other.tokens)
                .ok_or(BudgetError::Invalid)?,
            cost_micro_cny: self
                .cost_micro_cny
                .checked_sub(other.cost_micro_cny)
                .ok_or(BudgetError::Invalid)?,
        })
    }

    pub fn within(self, limit: Self) -> bool {
        self.attempts <= limit.attempts
            && self.tokens <= limit.tokens
            && self.cost_micro_cny <= limit.cost_micro_cny
    }

    pub fn reserve(self, quote: Self, limits: Self) -> Result<Self, BudgetError> {
        let charged = self.checked_add(quote)?;
        if !charged.within(limits) {
            return Err(BudgetError::Exhausted);
        }
        Ok(charged)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub contract: String,
    pub profile: String,
    pub provider: String,
    pub model: String,
    pub origin: String,
    pub thinking_enabled: bool,
    pub input_token_ceiling: u64,
    pub input_micro_cny: u64,
    pub output_micro_cny: u64,
    pub max_messages: usize,
    pub max_request_bytes: usize,
    #[serde(default)]
    pub embedding_provider: Option<String>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub embedding_origin: Option<String>,
    #[serde(default)]
    pub embedding_input_token_ceiling: Option<u64>,
    #[serde(default)]
    pub embedding_input_micro_cny: Option<u64>,
    #[serde(default)]
    pub embedding_max_request_bytes: Option<usize>,
    #[serde(default)]
    pub embedding_dimensions: Option<usize>,
    #[serde(default)]
    pub embedding_runtime_image: Option<String>,
    #[serde(default)]
    pub embedding_model_revision: Option<String>,
    #[serde(default)]
    pub embedding_probe_image: Option<String>,
    pub max_lifetime_seconds: i64,
    pub max_limits: Amount,
    pub operations: BTreeMap<String, u32>,
}

impl Profile {
    pub fn compiled() -> Self {
        Self::compiled_named("vision-journey-diagnostic-v1")
            .expect("valid compiled diagnostic profile")
    }

    pub fn compiled_named(name: &str) -> Result<Self, BudgetError> {
        let source = match name {
            "vision-journey-diagnostic-v1" => PROFILE_JSON,
            "four-layer-journey-diagnostic-v2" => MEMORY_PROFILE_JSON,
            "four-layer-journey-diagnostic-v3" => LOCAL_MEMORY_PROFILE_JSON,
            _ => return Err(BudgetError::Invalid),
        };
        serde_json::from_str(source).map_err(|_| BudgetError::Invalid)
    }

    fn identity(&self, operation: &str) -> Result<(&str, &str, &str), BudgetError> {
        if operation == "embedding" {
            Ok((
                self.embedding_provider
                    .as_deref()
                    .ok_or(BudgetError::Invalid)?,
                self.embedding_model
                    .as_deref()
                    .ok_or(BudgetError::Invalid)?,
                self.embedding_origin
                    .as_deref()
                    .ok_or(BudgetError::Invalid)?,
            ))
        } else {
            Ok((&self.provider, &self.model, &self.origin))
        }
    }

    pub fn quote(&self, operation: &str, output_limit: u32) -> Result<Amount, BudgetError> {
        let ceiling = self.operations.get(operation).ok_or(BudgetError::Invalid)?;
        if (operation == "embedding") != (output_limit == 0) || output_limit > *ceiling {
            return Err(BudgetError::Invalid);
        }
        let (input_ceiling, input_price, output_price) = self.pricing(operation)?;
        Self::consumption(
            input_ceiling,
            u64::from(output_limit),
            input_price,
            output_price,
        )
    }

    pub fn usage(
        &self,
        operation: &str,
        output_limit: u32,
        usage: &Settlement,
    ) -> Result<Amount, BudgetError> {
        self.quote(operation, output_limit)?;
        let (_, model, _) = self.identity(operation)?;
        let (input_ceiling, input_price, output_price) = self.pricing(operation)?;
        if usage.model != model
            || usage.input_tokens > input_ceiling
            || usage.output_tokens > u64::from(output_limit)
            || usage
                .cached_input_tokens
                .is_some_and(|cached| cached > usage.input_tokens)
        {
            return Err(BudgetError::Invalid);
        }
        Self::consumption(
            usage.input_tokens,
            usage.output_tokens,
            input_price,
            output_price,
        )
    }

    fn pricing(&self, operation: &str) -> Result<(u64, u64, u64), BudgetError> {
        if operation == "embedding" {
            Ok((
                self.embedding_input_token_ceiling
                    .ok_or(BudgetError::Invalid)?,
                self.embedding_input_micro_cny.ok_or(BudgetError::Invalid)?,
                0,
            ))
        } else {
            Ok((
                self.input_token_ceiling,
                self.input_micro_cny,
                self.output_micro_cny,
            ))
        }
    }

    fn consumption(
        input: u64,
        output: u64,
        input_price: u64,
        output_price: u64,
    ) -> Result<Amount, BudgetError> {
        Ok(Amount {
            attempts: 1,
            tokens: input.checked_add(output).ok_or(BudgetError::Invalid)?,
            cost_micro_cny: input
                .checked_mul(input_price)
                .and_then(|amount| {
                    output
                        .checked_mul(output_price)
                        .and_then(|output_cost| amount.checked_add(output_cost))
                })
                .ok_or(BudgetError::Invalid)?,
        })
    }
}

/// Persist all fields for idempotent comparison, not just the derived charge.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settlement {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cached_input_tokens: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_requires_rfc_v4_identity_and_fixed_bounded_expiry() {
        let profile = Profile::compiled();
        let now = DateTime::from_timestamp(1_800_000_000, 0).unwrap();
        let mut registration = Registration {
            budget_id: Uuid::new_v4(),
            contract: profile.contract,
            profile: profile.profile,
            profile_sha256: "a".repeat(64),
            limits: profile.max_limits,
            expires_at: now + chrono::Duration::hours(4),
        };
        assert_eq!(registration.validate_provision(now), Ok(()));
        assert_eq!(
            registration.validate_provision(now - chrono::Duration::nanoseconds(1)),
            Err(BudgetError::Invalid)
        );
        assert_eq!(
            registration.validate_provision(registration.expires_at),
            Err(BudgetError::Invalid)
        );
        assert_eq!(
            registration.validate(),
            Ok(()),
            "expired registrations remain readable"
        );
        // Version nibble alone is insufficient: this UUID has the NCS variant.
        let non_rfc = Uuid::parse_str("00000000-0000-4000-0000-000000000001").unwrap();
        assert_eq!(non_rfc.get_version(), Some(Version::Random));
        registration.budget_id = non_rfc;
        assert_eq!(registration.validate(), Err(BudgetError::Invalid));
        assert_eq!(
            Attempt {
                attempt_id: non_rfc,
                operation: "setup_connection".into(),
                output_limit: 8
            }
            .quote(),
            Err(BudgetError::Invalid)
        );
    }

    #[test]
    fn reservation_uses_server_profile_and_keeps_every_attempt_charged() {
        let profile = Profile::compiled();
        assert!(!profile.thinking_enabled);
        let quote = profile.quote("setup_connection", 8).unwrap();
        assert_eq!(
            quote,
            Amount {
                attempts: 1,
                tokens: 1_048_584,
                cost_micro_cny: 4_194_400
            }
        );
        assert_eq!(Amount::default().reserve(quote, quote), Ok(quote));
        assert_eq!(quote.reserve(quote, quote), Err(BudgetError::Exhausted));
        assert_eq!(
            Amount::default().reserve(quote, Amount::default()),
            Err(BudgetError::Exhausted)
        );
        assert_eq!(
            profile.quote("setup_connection", 9),
            Err(BudgetError::Invalid)
        );
        assert_eq!(profile.quote("unknown", 1), Err(BudgetError::Invalid));
        assert_eq!(
            profile.quote("character_chat", 0),
            Err(BudgetError::Invalid)
        );

        let usage = Settlement {
            model: profile.model.clone(),
            input_tokens: 10,
            output_tokens: 2,
            cached_input_tokens: Some(10),
        };
        let actual = profile.usage("setup_connection", 8, &usage).unwrap();
        assert_eq!(
            actual,
            Amount {
                attempts: 1,
                tokens: 12,
                cost_micro_cny: 64
            }
        );
        let charged = quote
            .checked_sub(quote)
            .unwrap()
            .checked_add(actual)
            .unwrap();
        assert_eq!(
            charged.attempts, 1,
            "settlement never refunds the attempt count"
        );
        assert_eq!(charged.reserve(quote, quote), Err(BudgetError::Exhausted));
    }

    #[test]
    fn memory_profile_prices_embedding_without_relaxing_v1() {
        let v1 = Profile::compiled();
        let v2 = Profile::compiled_named("four-layer-journey-diagnostic-v2").unwrap();
        let v3 = Profile::compiled_named("four-layer-journey-diagnostic-v3").unwrap();
        assert_eq!(v1.model, "deepseek-flash");
        assert_eq!(v2.model, "deepseek-v4-flash");
        assert_eq!(v3.model, "deepseek-v4-flash");
        assert_eq!(v1.quote("embedding", 0), Err(BudgetError::Invalid));
        assert_eq!(
            v2.quote("embedding", 0),
            Ok(Amount {
                attempts: 1,
                tokens: 8192,
                cost_micro_cny: 8192,
            })
        );
        assert_eq!(v2.quote("embedding", 1), Err(BudgetError::Invalid));
        assert_eq!(
            v3.quote("embedding", 0),
            Ok(Amount {
                attempts: 1,
                tokens: 8192,
                cost_micro_cny: 0,
            })
        );
        assert_eq!(
            v3.identity("embedding"),
            Ok((
                "local-tei",
                "Alibaba-NLP/gte-Qwen2-1.5B-instruct",
                "http://embedding:80"
            ))
        );
        assert_eq!(
            v3.embedding_runtime_image.as_deref(),
            Some("ghcr.io/huggingface/text-embeddings-inference:cpu-1.9.3@sha256:c26a226262ad4ff3330fb30b76653c1bb65da2fcf413b92284545a010e0a8a48")
        );
        assert_eq!(
            v3.embedding_model_revision.as_deref(),
            Some("a9af15a6372d7d6b25e9fb07c2ccb9e1fe645644")
        );
        assert_eq!(
            v3.embedding_probe_image.as_deref(),
            Some("nginx:alpine@sha256:db35bfc6b2951e7f8a72db5db120288c127ffaeeb4a6d4b95a26fead017d5913")
        );
        assert_eq!(
            llm_client::diagnostic_budget::profile_sha256_for("four-layer-journey-diagnostic-v2"),
            "6cee114e4008b250e2d00d629c938dd245027d5245437b1f2cbdeb81c4bdffbc"
        );
        assert_eq!(
            v2.usage(
                "embedding",
                0,
                &Settlement {
                    model: "text-embedding-3-small".into(),
                    input_tokens: 7,
                    output_tokens: 0,
                    cached_input_tokens: None,
                }
            ),
            Ok(Amount {
                attempts: 1,
                tokens: 7,
                cost_micro_cny: 7,
            })
        );
    }

    #[test]
    fn invalid_usage_and_integer_overflow_fail_closed() {
        let profile = Profile::compiled();
        let valid = Settlement {
            model: profile.model.clone(),
            input_tokens: 10,
            output_tokens: 2,
            cached_input_tokens: None,
        };
        for invalid in [
            Settlement {
                model: "other-model".into(),
                ..valid.clone()
            },
            Settlement {
                input_tokens: profile.input_token_ceiling + 1,
                ..valid.clone()
            },
            Settlement {
                output_tokens: 9,
                ..valid.clone()
            },
            Settlement {
                cached_input_tokens: Some(11),
                ..valid.clone()
            },
        ] {
            assert_eq!(
                profile.usage("setup_connection", 8, &invalid),
                Err(BudgetError::Invalid)
            );
        }
        let maximum = Amount {
            attempts: u64::MAX,
            tokens: u64::MAX,
            cost_micro_cny: u64::MAX,
        };
        let one = Amount {
            attempts: 1,
            tokens: 1,
            cost_micro_cny: 1,
        };
        assert_eq!(maximum.checked_add(one), Err(BudgetError::Invalid));
        assert_eq!(
            Amount::default().checked_sub(one),
            Err(BudgetError::Invalid)
        );
        let mut overflowing_profile = Profile::compiled();
        overflowing_profile.input_micro_cny = u64::MAX;
        assert_eq!(
            overflowing_profile.quote("setup_connection", 8),
            Err(BudgetError::Invalid)
        );
    }
}
