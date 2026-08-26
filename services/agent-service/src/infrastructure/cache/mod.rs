pub mod redis_cache;
pub use redis_cache::RedisCache;

use crate::domain::{
    entities::memory::ChatMessage,
    ports::{MessageCache, ReadinessProbe},
};
use async_trait::async_trait;
use deadpool_redis::Pool;
use std::time::Duration;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheMode {
    Postgres,
    Redis,
}

pub fn parse_cache_mode(value: Option<&str>) -> anyhow::Result<CacheMode> {
    match value.map(str::trim).filter(|value| !value.is_empty()) {
        None | Some("postgres") => Ok(CacheMode::Postgres),
        Some("redis") => Ok(CacheMode::Redis),
        Some(_) => anyhow::bail!("CACHE_MODE must be postgres or redis"),
    }
}

pub fn validate_redis_url(value: &str) -> anyhow::Result<()> {
    let url = redis::parse_redis_url(value).ok_or_else(|| {
        anyhow::anyhow!("REDIS_URL must be an absolute redis:// or rediss:// URL")
    })?;
    if url.host_str().is_none() {
        anyhow::bail!("REDIS_URL must include a host");
    }
    let password = url
        .password()
        .filter(|password| !password.is_empty())
        .ok_or_else(|| anyhow::anyhow!("REDIS_URL must include a password"))?;
    let lowered = password.to_ascii_lowercase();
    let mut seen = [false; 256];
    for byte in password.bytes() {
        seen[usize::from(byte)] = true;
    }
    if password.len() < 16
        || seen.into_iter().filter(|value| *value).count() < 8
        || lowered.contains("placeholder")
        || lowered.contains("change_me")
        || matches!(
            lowered.as_str(),
            "your_redis_password_here" | "runtime-redis-only"
        )
    {
        anyhow::bail!("REDIS_URL must include a strong non-placeholder password");
    }
    Ok(())
}

/// Desktop adapter: PostgreSQL remains authoritative, so skipping the
/// reconstructable recent-message projection is safe when Redis is absent.
pub struct NoopMessageCache;

#[async_trait]
impl MessageCache for NoopMessageCache {
    async fn push_turn(
        &self,
        _character_id: Uuid,
        _user_id: Uuid,
        _user_message: &ChatMessage,
        _character_message: &ChatMessage,
    ) -> anyhow::Result<bool> {
        Ok(true)
    }

    async fn clear(&self, _character_id: Uuid, _user_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
    async fn clear_user(&self, _user_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
    async fn clear_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
    async fn allow_user(&self, _user_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
    async fn allow_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> anyhow::Result<()> {
        Ok(())
    }
}

pub struct AlwaysReadyProbe;

#[async_trait]
impl ReadinessProbe for AlwaysReadyProbe {
    async fn is_ready(&self) -> bool {
        true
    }
}

pub struct RedisReadinessProbe {
    pool: Pool,
}

impl RedisReadinessProbe {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ReadinessProbe for RedisReadinessProbe {
    async fn is_ready(&self) -> bool {
        let check = async {
            match self.pool.get().await {
                Ok(mut connection) => redis::cmd("PING")
                    .query_async::<String>(&mut connection)
                    .await
                    .is_ok(),
                Err(_) => false,
            }
        };

        tokio::time::timeout(Duration::from_secs(2), check)
            .await
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_cache_mode, validate_redis_url, AlwaysReadyProbe, CacheMode};
    use crate::domain::ports::ReadinessProbe;

    #[tokio::test]
    async fn postgres_cache_mode_is_ready_without_redis() {
        assert!(AlwaysReadyProbe.is_ready().await);
    }

    #[test]
    fn cache_mode_is_explicit_and_postgres_is_the_default() {
        assert_eq!(parse_cache_mode(None).unwrap(), CacheMode::Postgres);
        assert_eq!(
            parse_cache_mode(Some("postgres")).unwrap(),
            CacheMode::Postgres
        );
        assert_eq!(parse_cache_mode(Some("redis")).unwrap(), CacheMode::Redis);
        assert!(parse_cache_mode(Some("memory")).is_err());
    }

    #[test]
    fn redis_mode_requires_an_authenticated_non_placeholder_url() {
        assert!(validate_redis_url("memory://").is_err());
        assert!(validate_redis_url("redis://redis:6379").is_err());
        assert!(validate_redis_url("redis://:your_redis_password_here@redis:6379").is_err());
        assert!(validate_redis_url("redis://:0123456789abcdef0123456789abcdef@redis:6379").is_ok());
    }
}
