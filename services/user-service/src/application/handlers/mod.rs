use std::sync::Arc;
use uuid::Uuid;

use crate::domain::entities::user::{RefreshToken, User};
use crate::domain::ports::AccessTokenIssuer;
use crate::domain::repositories::UserRepository;

const MAX_PASSWORD_BYTES: usize = 72;
const MAX_NAME_CHARS: usize = 200;
const REFRESH_TOKEN_BYTES: usize = 64;

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
    #[error("Invalid email or password")]
    InvalidCredentials,
    #[error("Invalid or expired refresh token")]
    InvalidRefreshToken,
    #[error("User not found")]
    NotFound,
    #[error("Authentication operation failed")]
    Internal(#[source] anyhow::Error),
}

pub type AuthResult<T> = std::result::Result<T, AuthError>;

pub struct AuthHandler {
    pub user_repo: Arc<dyn UserRepository>,
    pub jwt: Arc<dyn AccessTokenIssuer>,
    pub refresh_token_expiry: i64,
}

impl AuthHandler {
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

        let password_hash = bcrypt::hash(password, 12)
            .map_err(anyhow::Error::from)
            .map_err(AuthError::Internal)?;
        let user = User::new(email, password_hash, name);
        let (access_token, refresh_token) = self.issue_tokens(&user)?;
        if !self
            .user_repo
            .save(&user)
            .await
            .map_err(AuthError::Internal)?
        {
            return Err(AuthError::EmailAlreadyRegistered);
        }
        self.user_repo
            .save_refresh_token(&refresh_token)
            .await
            .map_err(AuthError::Internal)?;

        Ok((user, access_token, refresh_token.token))
    }

    #[tracing::instrument(skip(self, password))]
    pub async fn setup(
        &self,
        email: &str,
        password: &str,
        name: Option<String>,
    ) -> AuthResult<(User, String, String)> {
        let (email, name) = validate_registration(email, password, name)?;
        let password_hash = bcrypt::hash(password, 12)
            .map_err(anyhow::Error::from)
            .map_err(AuthError::Internal)?;
        let user = User::new_admin(email, password_hash, name);
        let (access_token, refresh_token) = self.issue_tokens(&user)?;

        if !self
            .user_repo
            .save_initial_user(&user, &refresh_token)
            .await
            .map_err(AuthError::Internal)?
        {
            return Err(AuthError::AlreadyConfigured);
        }

        Ok((user, access_token, refresh_token.token))
    }

    pub async fn is_configured(&self) -> AuthResult<bool> {
        self.user_repo.has_any().await.map_err(AuthError::Internal)
    }

    #[tracing::instrument(skip(self, password))]
    pub async fn login(&self, email: &str, password: &str) -> AuthResult<(User, String, String)> {
        let mut user = self
            .user_repo
            .find_by_email(email.trim())
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::InvalidCredentials)?;

        if !bcrypt::verify(password, &user.password_hash)
            .map_err(anyhow::Error::from)
            .map_err(AuthError::Internal)?
        {
            return Err(AuthError::InvalidCredentials);
        }

        user.record_sign_in();
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

    pub async fn refresh(&self, refresh_token: &str) -> AuthResult<String> {
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

        self.jwt
            .generate_token(user.id, &user.email, user.role.as_str())
            .map_err(AuthError::Internal)
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
    let email = email.trim().to_lowercase();
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

pub fn is_valid_email(email: &str) -> bool {
    let parts: Vec<&str> = email.splitn(2, '@').collect();
    parts.len() == 2
        && !parts[0].is_empty()
        && parts[1].contains('.')
        && !parts[1].starts_with('.')
        && !parts[1].ends_with('.')
        && email.len() <= 320
}

fn generate_refresh_token() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
