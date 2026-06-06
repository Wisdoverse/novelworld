use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::memory::ChatMessage;
use crate::domain::repositories::ChatRepository;

#[derive(Debug, FromRow)]
struct ChatMessageRow {
    id: Uuid,
    user_id: Uuid,
    character_id: Uuid,
    novel_id: Uuid,
    role: String,
    content: String,
    reader_identity: Option<String>,
    chapter_context: Option<i32>,
    created_at: DateTime<Utc>,
}

impl From<ChatMessageRow> for ChatMessage {
    fn from(r: ChatMessageRow) -> Self {
        ChatMessage {
            id: r.id,
            user_id: r.user_id,
            character_id: r.character_id,
            novel_id: r.novel_id,
            role: r.role,
            content: r.content,
            reader_identity: r.reader_identity,
            chapter_context: r.chapter_context,
            created_at: r.created_at,
        }
    }
}

pub struct PgChatRepository {
    pool: PgPool,
}

impl PgChatRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl ChatRepository for PgChatRepository {
    async fn save(&self, msg: &ChatMessage) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO chat_messages (
                id, user_id, character_id, novel_id,
                role, content, reader_identity, chapter_context, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(msg.id)
        .bind(msg.user_id)
        .bind(msg.character_id)
        .bind(msg.novel_id)
        .bind(&msg.role)
        .bind(&msg.content)
        .bind(&msg.reader_identity)
        .bind(msg.chapter_context)
        .bind(msg.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_recent(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        let rows = sqlx::query_as::<_, ChatMessageRow>(
            r#"
            SELECT id, user_id, character_id, novel_id,
                   role, content, reader_identity, chapter_context, created_at
            FROM chat_messages
            WHERE character_id = $1 AND user_id = $2 AND novel_id = $3
            ORDER BY created_at DESC
            LIMIT $4
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        // Reverse to get chronological order (oldest first)
        let mut messages: Vec<ChatMessage> = rows.into_iter().map(ChatMessage::from).collect();
        messages.reverse();
        Ok(messages)
    }

    async fn count(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<usize> {
        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*) FROM chat_messages
            WHERE character_id = $1 AND user_id = $2 AND novel_id = $3
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0 as usize)
    }

    async fn find_by_character_user(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChatMessage>> {
        let rows = sqlx::query_as::<_, ChatMessageRow>(
            r#"
            SELECT id, user_id, character_id, novel_id,
                   role, content, reader_identity, chapter_context, created_at
            FROM chat_messages
            WHERE character_id = $1 AND user_id = $2
            ORDER BY created_at DESC
            LIMIT $3 OFFSET $4
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(ChatMessage::from).collect())
    }
}
