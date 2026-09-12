use std::ffi::OsString;

use chrono::{DateTime, SecondsFormat, Utc};
pub use llm_client::diagnostic_budget::{profile_sha256, profile_sha256_for};
use serde::Deserialize;
use uuid::Uuid;

use crate::domain::entities::diagnostic_budget::{Amount, BudgetError, Profile, Registration};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Limits {
    profile: String,
    max_attempts: u64,
    max_tokens: u64,
    max_cost_micro_cny: u64,
    expires_at: String,
}

pub fn registration_from_environment() -> Result<Option<Registration>, BudgetError> {
    parse_registration(
        std::env::var_os("LLM_DIAGNOSTIC_BUDGET_ID"),
        std::env::var_os("LLM_DIAGNOSTIC_BUDGET_LIMITS"),
        std::env::var_os("LLM_DIAGNOSTIC_PROFILE"),
    )
}

fn parse_registration(
    id: Option<OsString>,
    limits: Option<OsString>,
    selector: Option<OsString>,
) -> Result<Option<Registration>, BudgetError> {
    let id = id
        .unwrap_or_default()
        .into_string()
        .map_err(|_| BudgetError::Invalid)?;
    let limits = limits
        .unwrap_or_default()
        .into_string()
        .map_err(|_| BudgetError::Invalid)?;
    let selector = selector
        .unwrap_or_else(|| "vision-journey-diagnostic-v1".into())
        .into_string()
        .map_err(|_| BudgetError::Invalid)?;
    if id.is_empty() && limits.is_empty() {
        return if selector == "vision-journey-diagnostic-v1" {
            Ok(None)
        } else {
            Err(BudgetError::Invalid)
        };
    }
    if id.len() != 36 || limits.is_empty() || limits.len() > 4096 {
        return Err(BudgetError::Invalid);
    }
    let budget_id = Uuid::parse_str(&id).map_err(|_| BudgetError::Invalid)?;
    if budget_id.to_string() != id {
        return Err(BudgetError::Invalid);
    }
    let limits: Limits = serde_json::from_str(&limits).map_err(|_| BudgetError::Invalid)?;
    let expires_at = DateTime::parse_from_rfc3339(&limits.expires_at)
        .map_err(|_| BudgetError::Invalid)?
        .with_timezone(&Utc);
    if expires_at.to_rfc3339_opts(SecondsFormat::Secs, true) != limits.expires_at {
        return Err(BudgetError::Invalid);
    }
    let registration = Registration {
        budget_id,
        contract: Profile::compiled_named(&selector)?.contract,
        profile: limits.profile,
        profile_sha256: profile_sha256_for(&selector),
        limits: Amount {
            attempts: limits.max_attempts,
            tokens: limits.max_tokens,
            cost_micro_cny: limits.max_cost_micro_cny,
        },
        expires_at,
    };
    if registration.profile != selector || registration.profile_sha256.is_empty() {
        return Err(BudgetError::Invalid);
    }
    registration.validate()?;
    Ok(Some(registration))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIMITS: &str = r#"{"profile":"vision-journey-diagnostic-v1","max_attempts":0,"max_tokens":0,"max_cost_micro_cny":0,"expires_at":"2026-09-07T12:00:00Z"}"#;
    const ID: &str = "53d07913-dc12-4821-942c-726deba253cb";

    #[test]
    fn diagnostic_registration_is_explicit_strict_and_compiled_profile_bound() {
        assert_eq!(parse_registration(None, None, None), Ok(None));
        assert_eq!(
            parse_registration(Some("".into()), Some("".into()), None),
            Ok(None)
        );
        let registration = parse_registration(Some(ID.into()), Some(LIMITS.into()), None)
            .unwrap()
            .unwrap();
        assert_eq!(registration.limits, Amount::default());
        assert_eq!(
            registration.profile_sha256,
            llm_client::diagnostic_budget::profile_sha256()
        );
        assert_eq!(
            crate::domain::entities::diagnostic_budget::PROFILE_JSON,
            llm_client::diagnostic_budget::PROFILE_JSON
        );
        assert_eq!(
            parse_registration(None, None, Some("four-layer-journey-diagnostic-v2".into())),
            Err(BudgetError::Invalid)
        );
        for (id, limits) in [
            ("".to_owned(), LIMITS.to_owned()),
            (ID.to_owned(), "".to_owned()),
            (ID.to_uppercase(), LIMITS.to_owned()),
            (ID.replace('-', ""), LIMITS.to_owned()),
            (
                ID.to_owned(),
                LIMITS.replace(
                    "\"max_attempts\":0",
                    "\"max_attempts\":0,\"max_attempts\":0",
                ),
            ),
            (
                ID.to_owned(),
                LIMITS.replace("\"max_attempts\":0", "\"max_attempts\":2001"),
            ),
            (
                ID.to_owned(),
                LIMITS.replace("\"max_attempts\":0", "\"max_attempts\":-1"),
            ),
            (
                ID.to_owned(),
                LIMITS.replace("\"max_attempts\":0", "\"extra\":0"),
            ),
            (ID.to_owned(), LIMITS.replace("12:00:00Z", "12:00:00.000Z")),
            (
                ID.to_owned(),
                LIMITS.replace("vision-journey-diagnostic-v1", "other"),
            ),
        ] {
            assert_eq!(
                parse_registration(Some(id.into()), Some(limits.into()), None),
                Err(BudgetError::Invalid)
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn diagnostic_non_unicode_environment_never_silently_disables_the_budget() {
        use std::os::unix::ffi::OsStringExt;
        let invalid = OsString::from_vec(vec![0xff]);
        assert_eq!(
            parse_registration(Some(invalid.clone()), None, None),
            Err(BudgetError::Invalid)
        );
        assert_eq!(
            parse_registration(None, Some(invalid), None),
            Err(BudgetError::Invalid)
        );
    }
}
