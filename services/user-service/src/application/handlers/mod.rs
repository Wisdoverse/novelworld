use std::sync::Arc;
use anyhow::{Result, anyhow};
use uuid::Uuid;

use crate::domain::entities::user::{User, RefreshToken};
use crate::domain::repositories::UserRepository;
use crate::infrastructure::auth::jwt::JwtService;

pub struct AuthHandler {
    pub user_repo: Arc<dyn UserRepository>,
    pub jwt: Arc<JwtService>,
    pub refresh_token_expiry: i64,
}

impl AuthHandler {
    pub async fn register(
        &self,
        email: &str,
        password: &str,
        name: Option<String>,
    ) -> Result<(User, String, String)> {
        if !is_valid_email(email) {
            return Err(anyhow!("Invalid email format"));
        }
        if password.len() < 8 {
            return Err(anyhow!("Password must be at least 8 characters"));
        }
        if self.user_repo.find_by_email(email).await?.is_some() {
            return Err(anyhow!("Email already registered"));
        }

        let password_hash = bcrypt::hash(password, 12)?;
        let user = User::new(email.to_string(), password_hash, name);
        self.user_repo.save(&user).await?;

        let access_token = self.jwt.generate_token(user.id, &user.email, user.role.as_str())?;
        let refresh_token_str = generate_refresh_token();
        let refresh_token = RefreshToken::new(user.id, refresh_token_str.clone(), self.refresh_token_expiry);
        self.user_repo.save_refresh_token(&refresh_token).await?;

        Ok((user, access_token, refresh_token_str))
    }

    pub async fn login(&self, email: &str, password: &str) -> Result<(User, String, String)> {
        let mut user = self.user_repo.find_by_email(email).await?
            .ok_or_else(|| anyhow!("Invalid email or password"))?;

        if !bcrypt::verify(password, &user.password_hash)? {
            return Err(anyhow!("Invalid email or password"));
        }

        user.record_sign_in();
        self.user_repo.update(&user).await?;

        let access_token = self.jwt.generate_token(user.id, &user.email, user.role.as_str())?;
        let refresh_token_str = generate_refresh_token();
        let refresh_token = RefreshToken::new(user.id, refresh_token_str.clone(), self.refresh_token_expiry);
        self.user_repo.save_refresh_token(&refresh_token).await?;

        Ok((user, access_token, refresh_token_str))
    }

    pub async fn refresh(&self, refresh_token: &str) -> Result<String> {
        let token = self.user_repo.find_refresh_token(refresh_token).await?
            .ok_or_else(|| anyhow!("Invalid refresh token"))?;

        if token.is_expired() {
            self.user_repo.delete_refresh_token(refresh_token).await?;
            return Err(anyhow!("Refresh token expired"));
        }

        let user = self.user_repo.find_by_id(token.user_id).await?
            .ok_or_else(|| anyhow!("User not found"))?;

        let access_token = self.jwt.generate_token(user.id, &user.email, user.role.as_str())?;
        Ok(access_token)
    }

    pub async fn logout(&self, refresh_token: &str) -> Result<()> {
        self.user_repo.delete_refresh_token(refresh_token).await?;
        Ok(())
    }

    pub async fn get_me(&self, user_id: Uuid) -> Result<User> {
        self.user_repo.find_by_id(user_id).await?
            .ok_or_else(|| anyhow!("User not found"))
    }
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
    use uuid::Uuid;
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}
