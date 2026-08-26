use anyhow::Result;
use async_trait::async_trait;
use deadpool_redis::Pool;
use redis::AsyncCommands;
use uuid::Uuid;

use crate::domain::entities::memory::ChatMessage;
use crate::domain::ports::MessageCache;

const MAX_CACHED_MESSAGES: isize = 50;
const PRIVACY_TOMBSTONE_SECONDS: u64 = 60 * 60;
const PUSH_TURN_SCRIPT: &str = r#"
if redis.call('EXISTS', KEYS[1]) == 1 or redis.call('EXISTS', KEYS[2]) == 1 then
    return 0
end
redis.call('LPUSH', KEYS[3], ARGV[1], ARGV[2])
redis.call('LTRIM', KEYS[3], 0, tonumber(ARGV[3]))
return 1
"#;

/// Redis-backed short-term message cache.
/// Uses LIST per character-user pair: key = `chat:{character_id}:{user_id}`.
/// Messages are stored as JSON strings, most-recent first.
pub struct RedisCache {
    pool: Pool,
}

impl RedisCache {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    fn cache_key(character_id: Uuid, user_id: Uuid) -> String {
        format!("chat:{}:{}", character_id, user_id)
    }

    fn user_tombstone(user_id: Uuid) -> String {
        format!("privacy:deleted:user:{user_id}")
    }

    fn novel_tombstone(user_id: Uuid, novel_id: Uuid) -> String {
        format!("privacy:deleted:user:{user_id}:novel:{novel_id}")
    }

    async fn set_tombstone(&self, key: String) -> Result<()> {
        let mut conn = self.pool.get().await?;
        redis::cmd("SET")
            .arg(key)
            .arg("1")
            .arg("EX")
            .arg(PRIVACY_TOMBSTONE_SECONDS)
            .query_async::<()>(&mut conn)
            .await?;
        Ok(())
    }

    async fn user_keys(&self, user_id: Uuid) -> Result<Vec<String>> {
        let mut conn = self.pool.get().await?;
        let pattern = format!("chat:*:{user_id}");
        let mut cursor = 0_u64;
        let mut keys = Vec::new();
        loop {
            let (next, mut batch): (u64, Vec<String>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(&pattern)
                .arg("COUNT")
                .arg(100)
                .query_async(&mut conn)
                .await?;
            keys.append(&mut batch);
            cursor = next;
            if cursor == 0 {
                break;
            }
        }
        // ponytail: materialize one user's cache keys; add Redis index sets only if measured cardinality makes this unsafe.
        Ok(keys)
    }

    async fn delete_keys(&self, keys: &[String]) -> Result<()> {
        let mut conn = self.pool.get().await?;
        for batch in keys.chunks(100) {
            redis::cmd("DEL")
                .arg(batch)
                .query_async::<usize>(&mut conn)
                .await?;
        }
        Ok(())
    }
}

#[async_trait]
impl MessageCache for RedisCache {
    /// Project one committed turn atomically, newest message first.
    async fn push_turn(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        user_message: &ChatMessage,
        character_message: &ChatMessage,
    ) -> Result<bool> {
        let mut conn = self.pool.get().await?;
        let key = Self::cache_key(character_id, user_id);
        let user_json = serde_json::to_string(user_message)?;
        let character_json = serde_json::to_string(character_message)?;

        let projected = redis::cmd("EVAL")
            .arg(PUSH_TURN_SCRIPT)
            .arg(3)
            .arg(Self::user_tombstone(user_id))
            .arg(Self::novel_tombstone(user_id, user_message.novel_id))
            .arg(&key)
            .arg(user_json)
            .arg(character_json)
            .arg(MAX_CACHED_MESSAGES - 1)
            .query_async::<i64>(&mut conn)
            .await?;

        Ok(projected == 1)
    }

    /// Clear all cached messages for a character-user pair.
    async fn clear(&self, character_id: Uuid, user_id: Uuid) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let key = Self::cache_key(character_id, user_id);
        conn.del::<_, ()>(&key).await?;
        Ok(())
    }

    async fn clear_user(&self, user_id: Uuid) -> Result<()> {
        self.set_tombstone(Self::user_tombstone(user_id)).await?;
        let keys = self.user_keys(user_id).await?;
        self.delete_keys(&keys).await
    }

    async fn clear_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        self.set_tombstone(Self::novel_tombstone(user_id, novel_id))
            .await?;
        let keys = self.user_keys(user_id).await?;
        let mut conn = self.pool.get().await?;
        let mut matching = Vec::new();
        for key in keys {
            let first: Vec<String> = conn.lrange(&key, 0, 0).await?;
            let belongs_to_novel = first.first().is_some_and(|value| {
                serde_json::from_str::<ChatMessage>(value)
                    .map(|message| message.novel_id == novel_id)
                    // Corrupt projections cannot be scoped safely; deleting them is safe because PostgreSQL is authoritative.
                    .unwrap_or(true)
            });
            if belongs_to_novel {
                matching.push(key);
            }
        }
        drop(conn);
        self.delete_keys(&matching).await
    }

    async fn allow_user(&self, user_id: Uuid) -> Result<()> {
        let mut conn = self.pool.get().await?;
        conn.del::<_, ()>(Self::user_tombstone(user_id)).await?;
        Ok(())
    }

    async fn allow_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        let mut conn = self.pool.get().await?;
        conn.del::<_, ()>(Self::novel_tombstone(user_id, novel_id))
            .await?;
        Ok(())
    }
}
