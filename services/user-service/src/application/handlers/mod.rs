use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::domain::entities::{
    runtime_config::RuntimeLlmConfig,
    user::{RefreshToken, User, UserRole},
};
use crate::domain::ports::{AccessTokenIssuer, LlmConnectionTester, PrivacyCleanupPort};
use crate::domain::repositories::{AccountDeletion, UserRepository, UserSave};

const MAX_PASSWORD_BYTES: usize = 72;
const MAX_NAME_CHARS: usize = 200;
const REFRESH_TOKEN_BYTES: usize = 64;
const DUMMY_PASSWORD_HASH: &str = "$2b$12$ll3uh.KED.qBVam69C6YoO.chFDr2ecTxuwqoopuPN69e0cmTYU92";

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("{0}")]
    Validation(String),
    #[error("Email already registered")]
    EmailAlreadyRegistered,
    #[error("Setup is already complete")]
    AlreadyConfigured,
    #[error("Initial setup is required")]
    SetupRequired,
    #[error("The AI provider could not be reached with these settings")]
    LlmUnavailable,
    #[error("Invalid email or password")]
    InvalidCredentials,
    #[error("Invalid or expired refresh token")]
    InvalidRefreshToken,
    #[error("User not found")]
    NotFound,
    #[error("The only administrator cannot be deleted while other users remain")]
    LastAdministrator,
    #[error("Account privacy cleanup is temporarily unavailable")]
    PrivacyCleanupUnavailable,
    #[error("Authentication capacity is busy")]
    Capacity,
    #[error("Authentication operation failed")]
    Internal(#[source] anyhow::Error),
}

pub type AuthResult<T> = std::result::Result<T, AuthError>;

pub struct AuthHandler {
    pub user_repo: Arc<dyn UserRepository>,
    pub jwt: Arc<dyn AccessTokenIssuer>,
    pub llm_tester: Arc<dyn LlmConnectionTester>,
    pub privacy_cleanup: Arc<dyn PrivacyCleanupPort>,
    pub environment_llm_config: Option<RuntimeLlmConfig>,
    pub refresh_token_expiry: i64,
    pub password_work: Arc<Semaphore>,
}

pub struct SetupStatus {
    pub admin_configured: bool,
    pub llm_configured: bool,
}

impl SetupStatus {
    pub fn configured(&self) -> bool {
        self.admin_configured && self.llm_configured
    }
}

impl AuthHandler {
    async fn hash_password(&self, password: &str) -> AuthResult<String> {
        let permit = self
            .password_work
            .clone()
            .try_acquire_owned()
            .map_err(|_| AuthError::Capacity)?;
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            bcrypt::hash(password, 12)
        })
        .await
        .map_err(|error| AuthError::Internal(error.into()))?
        .map_err(|error| AuthError::Internal(error.into()))
    }

    async fn verify_password(&self, password: &str, hash: &str) -> AuthResult<bool> {
        let permit = self
            .password_work
            .clone()
            .try_acquire_owned()
            .map_err(|_| AuthError::Capacity)?;
        let password = password.to_owned();
        let hash = hash.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            bcrypt::verify(password, &hash)
        })
        .await
        .map_err(|error| AuthError::Internal(error.into()))?
        .map_err(|error| AuthError::Internal(error.into()))
    }

    #[tracing::instrument(skip(self, password))]
    pub async fn register(
        &self,
        email: &str,
        password: &str,
        name: Option<String>,
    ) -> AuthResult<(User, String, String)> {
        let (email, name) = validate_registration(email, password, name)?;
        if !self
            .user_repo
            .has_any()
            .await
            .map_err(AuthError::Internal)?
        {
            return Err(AuthError::SetupRequired);
        }
        if self
            .user_repo
            .find_by_email(&email)
            .await
            .map_err(AuthError::Internal)?
            .is_some()
        {
            return Err(AuthError::EmailAlreadyRegistered);
        }

        let password_hash = self.hash_password(password).await?;
        let user = User::new(email, password_hash, name);
        let (access_token, refresh_token) = self.issue_tokens(&user)?;
        match self
            .user_repo
            .save(&user)
            .await
            .map_err(AuthError::Internal)?
        {
            UserSave::Saved => {}
            UserSave::EmailConflict => return Err(AuthError::EmailAlreadyRegistered),
            UserSave::SetupRequired => return Err(AuthError::SetupRequired),
        }
        self.user_repo
            .save_refresh_token(&refresh_token)
            .await
            .map_err(AuthError::Internal)?;

        Ok((user, access_token, refresh_token.token))
    }

    #[tracing::instrument(skip(self, password, api_key))]
    pub async fn setup(
        &self,
        email: &str,
        password: &str,
        name: Option<String>,
        provider: Option<&str>,
        api_key: Option<&str>,
    ) -> AuthResult<(User, String, String)> {
        let (email, name) = validate_registration(email, password, name)?;
        if self
            .user_repo
            .has_any()
            .await
            .map_err(AuthError::Internal)?
        {
            return Err(AuthError::AlreadyConfigured);
        }

        let runtime_config = if self.environment_llm_config.is_some() {
            None
        } else {
            let config = RuntimeLlmConfig::for_provider(
                provider.unwrap_or_default(),
                api_key.unwrap_or_default(),
            )
            .map_err(AuthError::Validation)?;
            if let Err(error) = self.llm_tester.test(&config).await {
                tracing::warn!(error = ?error, provider = %config.provider, "setup LLM connection test failed");
                return Err(AuthError::LlmUnavailable);
            }
            Some(config)
        };

        let password_hash = self.hash_password(password).await?;
        let user = User::new_admin(email, password_hash, name);
        let (access_token, refresh_token) = self.issue_tokens(&user)?;

        if !self
            .user_repo
            .save_initial_setup(&user, &refresh_token, runtime_config.as_ref())
            .await
            .map_err(AuthError::Internal)?
        {
            return Err(AuthError::AlreadyConfigured);
        }

        Ok((user, access_token, refresh_token.token))
    }

    pub async fn setup_status(&self) -> AuthResult<SetupStatus> {
        let admin_configured = self
            .user_repo
            .has_any()
            .await
            .map_err(AuthError::Internal)?;
        let llm_configured = self.environment_llm_config.is_some()
            || self
                .user_repo
                .find_runtime_llm_config()
                .await
                .map_err(AuthError::Internal)?
                .is_some();
        tracing::info!(admin_configured, llm_configured, "setup status served");
        Ok(SetupStatus {
            admin_configured,
            llm_configured,
        })
    }

    pub async fn runtime_llm_config(&self) -> AuthResult<RuntimeLlmConfig> {
        if let Some(config) = &self.environment_llm_config {
            return Ok(config.clone());
        }
        self.user_repo
            .find_runtime_llm_config()
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::SetupRequired)
    }

    pub async fn llm_settings(&self, user_id: Uuid) -> AuthResult<RuntimeLlmConfig> {
        self.require_admin(user_id).await?;
        self.runtime_llm_config().await
    }

    pub async fn update_llm_settings(
        &self,
        user_id: Uuid,
        provider: &str,
        model: &str,
        api_key: Option<&str>,
        thinking_enabled: bool,
    ) -> AuthResult<RuntimeLlmConfig> {
        self.require_admin(user_id).await?;
        if self.environment_llm_config.is_some() {
            return Err(AuthError::Validation(
                "LLM settings are managed by environment variables".into(),
            ));
        }
        let current = self.runtime_llm_config().await?;
        let key = api_key
            .map(str::trim)
            .filter(|key| !key.is_empty())
            .unwrap_or(&current.api_key);
        let config = RuntimeLlmConfig::for_settings(provider, model, key, thinking_enabled)
            .map_err(AuthError::Validation)?;
        self.llm_tester
            .test(&config)
            .await
            .map_err(|error| {
                tracing::warn!(error = ?error, provider = %config.provider, model = %config.model, "settings LLM connection test failed");
                AuthError::LlmUnavailable
            })?;
        self.user_repo
            .save_runtime_llm_config(&config)
            .await
            .map_err(AuthError::Internal)?;
        Ok(config)
    }

    async fn require_admin(&self, user_id: Uuid) -> AuthResult<User> {
        let user = self
            .user_repo
            .find_by_id(user_id)
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::NotFound)?;
        if user.role != UserRole::Admin {
            return Err(AuthError::Validation(
                "Administrator access is required".into(),
            ));
        }
        Ok(user)
    }

    #[tracing::instrument(skip(self, password))]
    pub async fn login(&self, email: &str, password: &str) -> AuthResult<(User, String, String)> {
        let user = self
            .user_repo
            .find_by_email(email.trim())
            .await
            .map_err(AuthError::Internal)?;
        let password_hash = user
            .as_ref()
            .map(|user| user.password_hash.as_str())
            .unwrap_or(DUMMY_PASSWORD_HASH);
        let valid = self.verify_password(password, password_hash).await?;
        let Some(mut user) = user.filter(|_| valid) else {
            return Err(AuthError::InvalidCredentials);
        };

        user.record_sign_in();
        tracing::info!(user_id = %user.id, "login succeeded");
        self.user_repo
            .update(&user)
            .await
            .map_err(AuthError::Internal)?;
        let (access_token, refresh_token) = self.issue_tokens(&user)?;
        self.user_repo
            .save_refresh_token(&refresh_token)
            .await
            .map_err(AuthError::Internal)?;

        Ok((user, access_token, refresh_token.token))
    }

    pub async fn refresh(&self, refresh_token: &str) -> AuthResult<(String, String)> {
        validate_refresh_token(refresh_token)?;
        let token = self
            .user_repo
            .find_refresh_token(refresh_token)
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::InvalidRefreshToken)?;

        if token.is_expired() {
            self.user_repo
                .delete_refresh_token(refresh_token)
                .await
                .map_err(AuthError::Internal)?;
            return Err(AuthError::InvalidRefreshToken);
        }

        let user = self
            .user_repo
            .find_by_id(token.user_id)
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::InvalidRefreshToken)?;

        let (access_token, replacement) = self.issue_tokens(&user)?;
        if !self
            .user_repo
            .rotate_refresh_token(refresh_token, &replacement)
            .await
            .map_err(AuthError::Internal)?
        {
            return Err(AuthError::InvalidRefreshToken);
        }
        Ok((access_token, replacement.token))
    }

    pub async fn logout(&self, refresh_token: &str) -> AuthResult<()> {
        validate_refresh_token(refresh_token)?;
        self.user_repo
            .delete_refresh_token(refresh_token)
            .await
            .map_err(AuthError::Internal)
    }

    pub async fn get_me(&self, user_id: Uuid) -> AuthResult<User> {
        self.user_repo
            .find_by_id(user_id)
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::NotFound)
    }

    pub async fn delete_account(&self, user_id: Uuid) -> AuthResult<()> {
        self.clear_user_cache(user_id).await?;
        let deletion = match self.user_repo.delete_account(user_id).await {
            Ok(deletion) => deletion,
            Err(error) => {
                self.allow_user_cache(user_id).await?;
                return Err(AuthError::Internal(error));
            }
        };
        match deletion {
            AccountDeletion::LastAdministrator => {
                self.allow_user_cache(user_id).await?;
                return Err(AuthError::LastAdministrator);
            }
            AccountDeletion::Deleted | AccountDeletion::AlreadyAbsent => {}
        }
        Ok(())
    }

    async fn clear_user_cache(&self, user_id: Uuid) -> AuthResult<()> {
        if let Err(error) = self.privacy_cleanup.clear_user(user_id).await {
            let _ = self.allow_user_cache(user_id).await;
            tracing::warn!(error = ?error, %user_id, "account privacy cleanup failed");
            return Err(AuthError::PrivacyCleanupUnavailable);
        }
        Ok(())
    }

    async fn allow_user_cache(&self, user_id: Uuid) -> AuthResult<()> {
        self.privacy_cleanup
            .allow_user(user_id)
            .await
            .map_err(|error| {
                tracing::warn!(error = ?error, %user_id, "account privacy cleanup rollback failed");
                AuthError::PrivacyCleanupUnavailable
            })
    }

    fn issue_tokens(&self, user: &User) -> AuthResult<(String, RefreshToken)> {
        let access_token = self
            .jwt
            .generate_token(user.id, &user.email, user.role.as_str())
            .map_err(AuthError::Internal)?;
        let refresh_token =
            RefreshToken::new(user.id, generate_refresh_token(), self.refresh_token_expiry);
        Ok((access_token, refresh_token))
    }
}

fn validate_registration(
    email: &str,
    password: &str,
    name: Option<String>,
) -> AuthResult<(String, Option<String>)> {
    // Gate non-ASCII on the raw input: RFC 5321 mailboxes are ASCII-only, and
    // lowercasing first would let Unicode case folds (e.g. U+212A -> 'k')
    // slip through and collide with genuine ASCII addresses.
    let email = email.trim();
    if !email.is_ascii() {
        return Err(AuthError::Validation("Invalid email format".into()));
    }
    let email = email.to_lowercase();
    if !is_valid_email(&email) {
        return Err(AuthError::Validation("Invalid email format".into()));
    }
    if password.chars().count() < 8 {
        return Err(AuthError::Validation(
            "Password must be at least 8 characters".into(),
        ));
    }
    if password.len() > MAX_PASSWORD_BYTES {
        return Err(AuthError::Validation(format!(
            "Password must not exceed {MAX_PASSWORD_BYTES} bytes"
        )));
    }

    let name = name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if name.as_ref().is_some_and(|value| {
        value.chars().count() > MAX_NAME_CHARS || value.chars().any(char::is_control)
    }) {
        return Err(AuthError::Validation(format!(
            "Name must not exceed {MAX_NAME_CHARS} characters or contain control characters"
        )));
    }
    Ok((email, name))
}

fn validate_refresh_token(token: &str) -> AuthResult<()> {
    if token.len() != REFRESH_TOKEN_BYTES || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AuthError::InvalidRefreshToken);
    }
    Ok(())
}

/// RFC 5321 §4.1.2 Mailbox syntax, ASCII-only (SMTPUTF8 is out of scope and
/// rejected). The 254-octet total cap follows §4.5.3.1.3: the 256-octet path
/// limit includes the surrounding angle brackets. This is syntactic validation
/// only — the spec requires no MX or delivery check.
pub fn is_valid_email(email: &str) -> bool {
    if email.len() > 254 || !email.is_ascii() {
        return false;
    }
    let bytes = email.as_bytes();
    let Some(split) = mailbox_separator(bytes) else {
        return false;
    };
    valid_local_part(&bytes[..split]) && valid_domain(&bytes[split + 1..])
}

/// Index of the @ separating local part and domain. A quoted local part may
/// itself contain @ (and escaped quotes), so the separator is the first @
/// outside the quoted string.
fn mailbox_separator(bytes: &[u8]) -> Option<usize> {
    if bytes.first() != Some(&b'"') {
        return bytes.iter().position(|byte| *byte == b'@');
    }
    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return (bytes.get(index + 1) == Some(&b'@')).then_some(index + 1),
            _ => index += 1,
        }
    }
    None
}

fn valid_local_part(local: &[u8]) -> bool {
    if local.is_empty() || local.len() > 64 {
        return false;
    }
    if local[0] == b'"' {
        // Quoted-string = DQUOTE *qcontentSMTP DQUOTE; quoted-pairSMTP is
        // backslash plus one printable ASCII byte.
        if local.len() < 2 || local[local.len() - 1] != b'"' {
            return false;
        }
        let content = &local[1..local.len() - 1];
        let mut index = 0;
        while index < content.len() {
            match content[index] {
                b'\\' => {
                    let escaped = content.get(index + 1);
                    if !escaped.is_some_and(|byte| (32..=126).contains(byte)) {
                        return false;
                    }
                    index += 2;
                }
                b'"' => return false,
                32..=33 | 35..=91 | 93..=126 => index += 1,
                _ => return false,
            }
        }
        return true;
    }
    // Dot-string = Atom *("." Atom): atoms separated by single dots, no
    // leading, trailing, or consecutive dots.
    let mut expect_atom = true;
    for &byte in local {
        if byte == b'.' {
            if expect_atom {
                return false;
            }
            expect_atom = true;
        } else if is_atext(byte) {
            expect_atom = false;
        } else {
            return false;
        }
    }
    !expect_atom
}

/// RFC 5322 §3.2.3 atext: ALPHA / DIGIT and the listed printable symbols.
fn is_atext(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'/'
                | b'='
                | b'?'
                | b'^'
                | b'_'
                | b'`'
                | b'{'
                | b'|'
                | b'}'
                | b'~'
        )
}

fn valid_domain(domain: &[u8]) -> bool {
    if domain.is_empty() || domain.len() > 255 {
        return false;
    }
    if domain[0] == b'[' {
        return valid_address_literal(domain);
    }
    // Domain = sub-domain *("." sub-domain); labels are let-dig-hyp, 1–63
    // octets, no leading or trailing hyphen.
    let mut label_start = 0;
    for (index, &byte) in domain.iter().enumerate() {
        if byte == b'.' {
            if !valid_label(&domain[label_start..index]) {
                return false;
            }
            label_start = index + 1;
        }
    }
    valid_label(&domain[label_start..])
}

fn valid_label(label: &[u8]) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label[0] != b'-'
        && label[label.len() - 1] != b'-'
        && label
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
}

/// RFC 5321 §4.1.3: Address-literal = "[" ( IPv4-address-literal /
/// IPv6-address-literal / General-address-literal ) "]".
fn valid_address_literal(literal: &[u8]) -> bool {
    if literal.len() < 3 || literal[literal.len() - 1] != b']' {
        return false;
    }
    let content = &literal[1..literal.len() - 1];
    if content.is_empty() {
        return false;
    }
    if let Some(ipv6) = content.strip_prefix(b"IPv6:") {
        return std::str::from_utf8(ipv6)
            .ok()
            .and_then(|value| value.parse::<std::net::Ipv6Addr>().ok())
            .is_some();
    }
    // IPv4-address-literal = Snum 3("." Snum); Snum = 1*3DIGIT, value
    // <= 255 (leading zeros permitted). The RFC 5321 general-address-literal
    // (unknown standardized tags) is deliberately rejected: ponytail: no
    // registered tag besides IPv6 is used in practice, so that grammar
    // branch has no working form to support.
    let groups: Vec<&[u8]> = content.split(|byte| *byte == b'.').collect();
    groups.len() == 4
        && groups.iter().all(|group| {
            !group.is_empty()
                && group.len() <= 3
                && group.iter().all(u8::is_ascii_digit)
                && std::str::from_utf8(group)
                    .ok()
                    .and_then(|group| group.parse::<u16>().ok())
                    .is_some_and(|value| value <= 255)
        })
}

fn generate_refresh_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

#[cfg(test)]
mod email_tests {
    use super::validate_registration;

    #[test]
    fn non_ascii_email_is_rejected_before_lowercasing() {
        // U+212A KELVIN SIGN lowercases to ASCII 'k'; it must be rejected on
        // the raw input instead of registering as k@example.com.
        assert!(validate_registration("\u{212a}@example.com", "password123", None).is_err());
        assert!(validate_registration("user@example.com", "password123", None).is_ok());
    }
}

#[cfg(test)]
mod password_tests {
    use super::*;

    #[test]
    fn dummy_password_hash_is_a_valid_cost_twelve_hash() {
        assert!(DUMMY_PASSWORD_HASH.starts_with("$2b$12$"));
        assert!(!bcrypt::verify("not-the-dummy-password", DUMMY_PASSWORD_HASH).unwrap());
    }
}
