use async_trait::async_trait;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::user::{User, RefreshToken};

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn save(&self, user: &User) -> Result<()>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>>;
    async fn find_by_email(&self, email: &str) -> Result<Option<User>>;
    async fn update(&self, user: &User) -> Result<()>;
    async fn save_refresh_token(&self, token: &RefreshToken) -> Result<()>;
    async fn find_refresh_token(&self, token: &str) -> Result<Option<RefreshToken>>;
    async fn delete_refresh_token(&self, token: &str) -> Result<()>;
    async fn delete_refresh_tokens_for_user(&self, user_id: Uuid) -> Result<()>;
}
