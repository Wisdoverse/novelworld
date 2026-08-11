use anyhow::Result;
use async_trait::async_trait;
use deadpool_redis::Pool;
use redis::AsyncCommands;
use uuid::Uuid;

use crate::domain::entities::memory::ChatMessage;
use crate::domain::ports::MessageCache;

const MAX_CACHED_MESSAGES: isize = 50;

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
}

#[async_trait]
impl MessageCache for RedisCache {
    /// Retrieve the most recent `limit` messages from Redis cache.
    async fn get_recent_messages(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        max_chapter: i32,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        let mut conn = self.pool.get().await?;
        let key = Self::cache_key(character_id, user_id);

        let raw: Vec<String> = conn.lrange(&key, 0, MAX_CACHED_MESSAGES - 1).await?;

        let messages: Vec<ChatMessage> = raw
            .into_iter()
            .filter_map(|s| serde_json::from_str(&s).ok())
            .filter(|message: &ChatMessage| {
                message
                    .chapter_context
                    .is_some_and(|chapter| chapter <= max_chapter)
            })
            .take(limit)
            .collect();

        // Redis stores most-recent first (LPUSH), but we want chronological order
        let mut messages = messages;
        messages.reverse();
        Ok(messages)
    }

    /// Project one committed turn atomically, newest message first.
    async fn push_turn(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        user_message: &ChatMessage,
        character_message: &ChatMessage,
    ) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let key = Self::cache_key(character_id, user_id);
        let user_json = serde_json::to_string(user_message)?;
        let character_json = serde_json::to_string(character_message)?;

        redis::pipe()
            .atomic()
            .cmd("LPUSH")
            .arg(&key)
            .arg(user_json)
            .arg(character_json)
            .ignore()
            .cmd("LTRIM")
            .arg(&key)
            .arg(0)
            .arg(MAX_CACHED_MESSAGES - 1)
            .ignore()
            .query_async::<()>(&mut conn)
            .await?;

        Ok(())
    }

    /// Clear all cached messages for a character-user pair.
    async fn clear(&self, character_id: Uuid, user_id: Uuid) -> Result<()> {
        let mut conn = self.pool.get().await?;
        let key = Self::cache_key(character_id, user_id);
        conn.del::<_, ()>(&key).await?;
        Ok(())
    }
}
