use anyhow::{bail, Result};

pub fn validate_jwt_secret(value: &str) -> Result<()> {
    validate_secret("JWT_SECRET", value)
}

pub fn validate_internal_service_token(value: &str) -> Result<()> {
    validate_secret("INTERNAL_SERVICE_TOKEN", value)?;
    if !value.bytes().all(|byte| byte.is_ascii_graphic()) {
        bail!("INTERNAL_SERVICE_TOKEN must contain visible ASCII characters only");
    }
    Ok(())
}

fn validate_secret(name: &str, value: &str) -> Result<()> {
    let mut seen = [false; 256];
    for byte in value.bytes() {
        seen[usize::from(byte)] = true;
    }
    let distinct = seen.into_iter().filter(|seen| *seen).count();
    let lowered = value.to_ascii_lowercase();
    if value.len() < 32
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
        || distinct < 8
        || lowered.contains("placeholder")
        || lowered.contains("change_me")
        || lowered.contains("runtime-smoke")
    {
        bail!("{name} must be a strong non-placeholder value of at least 32 characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_secrets_reject_weak_and_repository_placeholder_values() {
        assert!(validate_jwt_secret("short").is_err());
        assert!(validate_jwt_secret(&"a".repeat(64)).is_err());
        for value in [
            "change_me_to_a_random_32_char_string",
            "runtime-smoke-secret-at-least-32-characters",
            "manifest-placeholder-internal-token-at-least-32-characters",
        ] {
            assert!(validate_jwt_secret(value).is_err());
            assert!(validate_internal_service_token(value).is_err());
        }
        assert!(validate_internal_service_token(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        )
        .is_ok());
        assert!(validate_internal_service_token("0123456789abcdef0123456789abcde🙂").is_err());
        assert!(validate_internal_service_token("0123456789abcdef 0123456789abcdef").is_err());
    }
}
