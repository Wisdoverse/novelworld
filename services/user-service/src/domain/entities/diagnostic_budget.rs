use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::{Uuid, Variant, Version};

pub const PROFILE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tools/llm-budget/diagnostic-v1.json"
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
        let profile = Profile::compiled();
        if self.budget_id.get_version() != Some(Version::Random)
            || self.budget_id.get_variant() != Variant::RFC4122
            || self.contract != profile.contract
            || self.profile != profile.profile
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
            || remaining > chrono::Duration::seconds(Profile::compiled().max_lifetime_seconds)
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
    pub fn validate(&self) -> Result<(), BudgetError> {
        let profile = Profile::compiled();
        if self.provider != profile.provider
            || self.model != profile.model
            || self.origin != profile.origin
        {
            return Err(BudgetError::Invalid);
        }
        self.attempt.quote()?;
        Ok(())
    }
}

impl Attempt {
    pub fn quote(&self) -> Result<Amount, BudgetError> {
        if self.attempt_id.get_version() != Some(Version::Random)
            || self.attempt_id.get_variant() != Variant::RFC4122
        {
            return Err(BudgetError::Invalid);
        }
        Profile::compiled().quote(&self.operation, self.output_limit)
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
    pub max_lifetime_seconds: i64,
    pub max_limits: Amount,
    pub operations: BTreeMap<String, u32>,
}

impl Profile {
    pub fn compiled() -> Self {
        serde_json::from_str(PROFILE_JSON).expect("compiled diagnostic profile is valid")
    }

    pub fn quote(&self, operation: &str, output_limit: u32) -> Result<Amount, BudgetError> {
        let ceiling = self.operations.get(operation).ok_or(BudgetError::Invalid)?;
        if output_limit == 0 || output_limit > *ceiling {
            return Err(BudgetError::Invalid);
        }
        self.consumption(self.input_token_ceiling, u64::from(output_limit))
    }

    pub fn usage(
        &self,
        operation: &str,
        output_limit: u32,
        usage: &Settlement,
    ) -> Result<Amount, BudgetError> {
        self.quote(operation, output_limit)?;
        if usage.model != self.model
            || usage.input_tokens > self.input_token_ceiling
            || usage.output_tokens > u64::from(output_limit)
            || usage
                .cached_input_tokens
                .is_some_and(|cached| cached > usage.input_tokens)
        {
            return Err(BudgetError::Invalid);
        }
        self.consumption(usage.input_tokens, usage.output_tokens)
    }

    fn consumption(&self, input: u64, output: u64) -> Result<Amount, BudgetError> {
        Ok(Amount {
            attempts: 1,
            tokens: input.checked_add(output).ok_or(BudgetError::Invalid)?,
            cost_micro_cny: input
                .checked_mul(self.input_micro_cny)
                .and_then(|amount| {
                    output
                        .checked_mul(self.output_micro_cny)
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
