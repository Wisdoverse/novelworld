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

/// Desktop adapter: PostgreSQL remains authoritative, so skipping the
/// reconstructable recent-message projection is safe when Redis is absent.
pub struct NoopMessageCache;

#[async_trait]
impl MessageCache for NoopMessageCache {
    async fn get_recent_messages(
        &self,
        _character_id: Uuid,
        _user_id: Uuid,
        _max_chapter: i32,
        _limit: usize,
    ) -> anyhow::Result<Vec<ChatMessage>> {
        Ok(Vec::new())
    }

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
    use super::{AlwaysReadyProbe, NoopMessageCache};
    use crate::domain::{
        entities::memory::ChatMessage,
        ports::{MessageCache, ReadinessProbe},
    };
    use uuid::Uuid;

    #[tokio::test]
    async fn desktop_cache_falls_back_to_the_authoritative_store() {
        let cache = NoopMessageCache;
        let id = Uuid::new_v4();
        assert!(cache
            .get_recent_messages(id, id, 1, 10)
            .await
            .unwrap()
            .is_empty());
        assert!(AlwaysReadyProbe.is_ready().await);
        let _type_check: Option<ChatMessage> = None;
    }
}
