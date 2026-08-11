pub mod redis_cache;
pub use redis_cache::RedisCache;

use crate::domain::ports::ReadinessProbe;
use async_trait::async_trait;
use deadpool_redis::Pool;
use std::time::Duration;

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
