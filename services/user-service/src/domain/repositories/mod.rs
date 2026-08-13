use anyhow::Result;
use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::entities::runtime_config::RuntimeLlmConfig;
use crate::domain::entities::user::{RefreshToken, User};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountDeletion {
    Deleted,
    AlreadyAbsent,
    LastAdministrator,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserSave {
    Saved,
    EmailConflict,
    SetupRequired,
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn save(&self, user: &User) -> Result<UserSave>;
    async fn save_initial_setup(
        &self,
        user: &User,
        token: &RefreshToken,
        llm: Option<&RuntimeLlmConfig>,
    ) -> Result<bool>;
    async fn has_any(&self) -> Result<bool>;
    async fn find_runtime_llm_config(&self) -> Result<Option<RuntimeLlmConfig>>;
    async fn save_runtime_llm_config(&self, config: &RuntimeLlmConfig) -> Result<()>;
    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>>;
    async fn find_by_email(&self, email: &str) -> Result<Option<User>>;
    async fn update(&self, user: &User) -> Result<()>;
    async fn save_refresh_token(&self, token: &RefreshToken) -> Result<()>;
    async fn find_refresh_token(&self, token: &str) -> Result<Option<RefreshToken>>;
    async fn delete_refresh_token(&self, token: &str) -> Result<()>;
    async fn delete_refresh_tokens_for_user(&self, user_id: Uuid) -> Result<()>;
    async fn delete_account(&self, user_id: Uuid) -> Result<AccountDeletion>;
}
