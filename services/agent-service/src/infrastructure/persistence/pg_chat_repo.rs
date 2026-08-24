use anyhow::{bail, ensure, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::entities::memory::ChatMessage;
use crate::domain::repositories::{BeginChatTurn, ChatRepository, ChatTurnClaim};

const ACTIVE_TURN_INDEX: &str = "idx_chat_turns_one_in_progress";

fn is_active_turn_conflict(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database) if database.constraint() == Some(ACTIVE_TURN_INDEX))
}

#[derive(Debug, FromRow)]
struct ChatMessageRow {
    id: Uuid,
    turn_id: Option<Uuid>,
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
            turn_id: r.turn_id,
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

#[derive(Debug, FromRow)]
struct ChatTurnRow {
    user_id: Uuid,
    character_id: Uuid,
    novel_id: Uuid,
    request_fingerprint: Vec<u8>,
    chapter_context: i32,
    reader_identity: Option<String>,
    reader_identity_type: String,
    reader_character_id: Option<Uuid>,
    deviation_mode: String,
    status: String,
    attempt: i64,
    failure_code: Option<String>,
    lease_expired: bool,
    response: Option<String>,
}

impl ChatTurnRow {
    fn matches_request(&self, claim: &ChatTurnClaim) -> bool {
        self.user_id == claim.user_id
            && self.character_id == claim.character_id
            && self.novel_id == claim.novel_id
            && self.request_fingerprint == claim.request_fingerprint
    }

    fn claim(&self, id: Uuid) -> ChatTurnClaim {
        ChatTurnClaim {
            id,
            user_id: self.user_id,
            character_id: self.character_id,
            novel_id: self.novel_id,
            request_fingerprint: self.request_fingerprint.clone(),
            chapter_context: self.chapter_context,
            reader_identity: self.reader_identity.clone(),
            reader_identity_type: self.reader_identity_type.clone(),
            reader_character_id: self.reader_character_id,
            deviation_mode: self.deviation_mode.clone(),
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

    async fn supersede_expired_turn(&self, claim: &ChatTurnClaim) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE chat_turns
            SET status = 'failed', lease_expires_at = NULL,
                failure_code = 'superseded', updated_at = NOW()
            WHERE user_id = $1 AND character_id = $2 AND novel_id = $3
              AND status = 'in_progress' AND lease_expires_at <= NOW()
            "#,
        )
        .bind(claim.user_id)
        .bind(claim.character_id)
        .bind(claim.novel_id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn active_retry_after(&self, claim: &ChatTurnClaim) -> Result<u64> {
        let seconds: Option<i64> = sqlx::query_scalar(
            r#"
            SELECT GREATEST(
                       1,
                       CEIL(EXTRACT(EPOCH FROM lease_expires_at - NOW()))::BIGINT
                   )
            FROM chat_turns
            WHERE user_id = $1 AND character_id = $2 AND novel_id = $3
              AND status = 'in_progress'
            LIMIT 1
            "#,
        )
        .bind(claim.user_id)
        .bind(claim.character_id)
        .bind(claim.novel_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(seconds.unwrap_or(1).max(1) as u64)
    }
}

#[async_trait]
impl ChatRepository for PgChatRepository {
    async fn begin_turn(&self, claim: &ChatTurnClaim) -> Result<BeginChatTurn> {
        let mut inserted = None;
        for _ in 0..2 {
            match sqlx::query(
                r#"
                INSERT INTO chat_turns (
                    id, user_id, character_id, novel_id, request_fingerprint,
                    chapter_context, reader_identity, reader_identity_type,
                    reader_character_id, deviation_mode, status, attempt,
                    lease_expires_at
                ) VALUES (
                    $1, $2, $3, $4, $5, $6, $7, $8::identity_type,
                    $9, $10::deviation_mode, 'in_progress', 1,
                    NOW() + INTERVAL '2 minutes'
                )
                ON CONFLICT (id) DO NOTHING
                "#,
            )
            .bind(claim.id)
            .bind(claim.user_id)
            .bind(claim.character_id)
            .bind(claim.novel_id)
            .bind(&claim.request_fingerprint)
            .bind(claim.chapter_context)
            .bind(&claim.reader_identity)
            .bind(&claim.reader_identity_type)
            .bind(claim.reader_character_id)
            .bind(&claim.deviation_mode)
            .execute(&self.pool)
            .await
            {
                Ok(result) => {
                    inserted = Some(result);
                    break;
                }
                Err(error) if is_active_turn_conflict(&error) => {
                    if !self.supersede_expired_turn(claim).await? {
                        return Ok(BeginChatTurn::InProgress {
                            retry_after_seconds: self.active_retry_after(claim).await?,
                        });
                    }
                }
                Err(error) => return Err(error.into()),
            }
        }
        let Some(inserted) = inserted else {
            return Ok(BeginChatTurn::InProgress {
                retry_after_seconds: self.active_retry_after(claim).await?,
            });
        };

        if inserted.rows_affected() == 1 {
            return Ok(BeginChatTurn::Acquired {
                claim: claim.clone(),
                attempt: 1,
            });
        }

        for _ in 0..2 {
            let row = sqlx::query_as::<_, ChatTurnRow>(
                r#"
                SELECT turn.user_id, turn.character_id, turn.novel_id,
                       turn.request_fingerprint, turn.chapter_context,
                       turn.reader_identity, turn.reader_identity_type::text AS reader_identity_type,
                       turn.reader_character_id, turn.deviation_mode::text AS deviation_mode,
                       turn.status, turn.attempt, turn.failure_code,
                       COALESCE(turn.lease_expires_at <= NOW(), FALSE) AS lease_expired,
                       response.content AS response
                FROM chat_turns AS turn
                LEFT JOIN chat_messages AS response
                  ON response.turn_id = turn.id AND response.role = 'character'
                WHERE turn.id = $1
                "#,
            )
            .bind(claim.id)
            .fetch_one(&self.pool)
            .await?;

            if !row.matches_request(claim) {
                return Ok(BeginChatTurn::Conflict);
            }
            let persisted_claim = row.claim(claim.id);
            if row.status == "completed" {
                return Ok(BeginChatTurn::Completed {
                    claim: persisted_claim,
                    response: row.response.ok_or_else(|| {
                        anyhow::anyhow!("completed chat turn is missing its response")
                    })?,
                });
            }
            if row.status == "in_progress" && !row.lease_expired {
                return Ok(BeginChatTurn::InProgress {
                    retry_after_seconds: self.active_retry_after(claim).await?,
                });
            }
            if row.status == "failed" && row.failure_code.as_deref() == Some("superseded") {
                return Ok(BeginChatTurn::Conflict);
            }
            if row.status != "failed" && row.status != "in_progress" {
                bail!("unknown chat turn status");
            }

            let reclaimed: Option<(i64,)> = match sqlx::query_as(
                r#"
                UPDATE chat_turns
                SET status = 'in_progress', attempt = attempt + 1,
                    lease_expires_at = NOW() + INTERVAL '2 minutes',
                    failure_code = NULL, updated_at = NOW()
                WHERE id = $1 AND attempt = $2
                  AND ((status = 'failed' AND failure_code <> 'superseded')
                       OR (status = 'in_progress' AND lease_expires_at <= NOW()))
                RETURNING attempt
                "#,
            )
            .bind(claim.id)
            .bind(row.attempt)
            .fetch_optional(&self.pool)
            .await
            {
                Ok(reclaimed) => reclaimed,
                Err(error) if is_active_turn_conflict(&error) => {
                    if self.supersede_expired_turn(claim).await? {
                        continue;
                    }
                    return Ok(BeginChatTurn::InProgress {
                        retry_after_seconds: self.active_retry_after(claim).await?,
                    });
                }
                Err(error) => return Err(error.into()),
            };
            if let Some((attempt,)) = reclaimed {
                return Ok(BeginChatTurn::Acquired {
                    claim: persisted_claim,
                    attempt,
                });
            }
        }

        Ok(BeginChatTurn::InProgress {
            retry_after_seconds: self.active_retry_after(claim).await?,
        })
    }

    async fn renew_turn(&self, turn_id: Uuid, attempt: i64) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE chat_turns
            SET lease_expires_at = NOW() + INTERVAL '2 minutes', updated_at = NOW()
            WHERE id = $1 AND attempt = $2 AND status = 'in_progress'
            "#,
        )
        .bind(turn_id)
        .bind(attempt)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn complete_turn(
        &self,
        claim: &ChatTurnClaim,
        attempt: i64,
        user_message: &ChatMessage,
        character_message: &ChatMessage,
    ) -> Result<()> {
        ensure!(user_message.turn_id == Some(claim.id));
        ensure!(character_message.turn_id == Some(claim.id));
        ensure!(user_message.role == "user");
        ensure!(character_message.role == "character");
        for message in [user_message, character_message] {
            ensure!(message.user_id == claim.user_id);
            ensure!(message.character_id == claim.character_id);
            ensure!(message.novel_id == claim.novel_id);
            ensure!(message.chapter_context == Some(claim.chapter_context));
            ensure!(message.reader_identity == claim.reader_identity);
        }

        let mut transaction = self.pool.begin().await?;
        let claimed = sqlx::query(
            r#"
            UPDATE chat_turns
            SET status = 'completed', lease_expires_at = NULL,
                failure_code = NULL, completed_at = NOW(), updated_at = NOW()
            WHERE id = $1 AND attempt = $2 AND status = 'in_progress'
              AND user_id = $3 AND character_id = $4 AND novel_id = $5
              AND request_fingerprint = $6 AND chapter_context = $7
              AND reader_identity IS NOT DISTINCT FROM $8
              AND reader_identity_type = $9::identity_type
              AND reader_character_id IS NOT DISTINCT FROM $10
              AND deviation_mode = $11::deviation_mode
            "#,
        )
        .bind(claim.id)
        .bind(attempt)
        .bind(claim.user_id)
        .bind(claim.character_id)
        .bind(claim.novel_id)
        .bind(&claim.request_fingerprint)
        .bind(claim.chapter_context)
        .bind(&claim.reader_identity)
        .bind(&claim.reader_identity_type)
        .bind(claim.reader_character_id)
        .bind(&claim.deviation_mode)
        .execute(&mut *transaction)
        .await?;
        ensure!(claimed.rows_affected() == 1, "chat turn claim was fenced");

        for message in [user_message, character_message] {
            sqlx::query(
                r#"
                INSERT INTO chat_messages (
                    id, turn_id, user_id, character_id, novel_id,
                    role, content, reader_identity, chapter_context, created_at
                ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                "#,
            )
            .bind(message.id)
            .bind(message.turn_id)
            .bind(message.user_id)
            .bind(message.character_id)
            .bind(message.novel_id)
            .bind(&message.role)
            .bind(&message.content)
            .bind(&message.reader_identity)
            .bind(message.chapter_context)
            .bind(message.created_at)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(())
    }

    async fn fail_turn(&self, turn_id: Uuid, attempt: i64, failure_code: &str) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE chat_turns
            SET status = 'failed', lease_expires_at = NULL,
                failure_code = $3, updated_at = NOW()
            WHERE id = $1 AND attempt = $2 AND status = 'in_progress'
            "#,
        )
        .bind(turn_id)
        .bind(attempt)
        .bind(failure_code)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn find_recent(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        reader_character_id: Option<Uuid>,
        max_chapter: i32,
        limit: usize,
    ) -> Result<Vec<ChatMessage>> {
        let rows = sqlx::query_as::<_, ChatMessageRow>(
            r#"
            SELECT message.id, message.turn_id, message.user_id,
                   message.character_id, message.novel_id, message.role,
                   message.content, message.reader_identity,
                   message.chapter_context, message.created_at
            FROM chat_messages AS message
            LEFT JOIN chat_turns AS turn ON turn.id = message.turn_id
            WHERE message.character_id = $1
              AND message.user_id = $2
              AND message.novel_id = $3
              AND (
                    ($4::uuid IS NULL AND (
                        turn.id IS NULL
                        OR (turn.status = 'completed'
                            AND turn.reader_identity_type = 'self'
                            AND turn.reader_character_id IS NULL)
                    ))
                    OR ($4::uuid IS NOT NULL
                        AND turn.status = 'completed'
                        AND turn.reader_identity_type = 'character'
                        AND turn.reader_character_id = $4)
              )
              AND message.chapter_context IS NOT NULL
              AND message.chapter_context <= $5
            ORDER BY message.created_at DESC,
                     message.turn_id DESC NULLS LAST,
                     CASE message.role
                         WHEN 'character' THEN 0
                         WHEN 'user' THEN 1
                         ELSE 2
                     END,
                     message.id DESC
            LIMIT $6
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(reader_character_id)
        .bind(max_chapter)
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
        reader_character_id: Option<Uuid>,
    ) -> Result<usize> {
        let row: (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM chat_messages AS message
            LEFT JOIN chat_turns AS turn ON turn.id = message.turn_id
            WHERE message.character_id = $1
              AND message.user_id = $2
              AND message.novel_id = $3
              AND (
                    ($4::uuid IS NULL AND (
                        turn.id IS NULL
                        OR (turn.status = 'completed'
                            AND turn.reader_identity_type = 'self'
                            AND turn.reader_character_id IS NULL)
                    ))
                    OR ($4::uuid IS NOT NULL
                        AND turn.status = 'completed'
                        AND turn.reader_identity_type = 'character'
                        AND turn.reader_character_id = $4)
              )
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(reader_character_id)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.0 as usize)
    }

    #[allow(clippy::too_many_arguments)]
    async fn find_by_character_user(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        reader_character_id: Option<Uuid>,
        max_chapter: i32,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChatMessage>> {
        let rows = sqlx::query_as::<_, ChatMessageRow>(
            r#"
            SELECT message.id, message.turn_id, message.user_id,
                   message.character_id, message.novel_id, message.role,
                   message.content, message.reader_identity,
                   message.chapter_context, message.created_at
            FROM chat_messages AS message
            LEFT JOIN chat_turns AS turn ON turn.id = message.turn_id
            WHERE message.character_id = $1
              AND message.user_id = $2
              AND message.novel_id = $3
              AND (
                    ($4::uuid IS NULL AND (
                        turn.id IS NULL
                        OR (turn.status = 'completed'
                            AND turn.reader_identity_type = 'self'
                            AND turn.reader_character_id IS NULL)
                    ))
                    OR ($4::uuid IS NOT NULL
                        AND turn.status = 'completed'
                        AND turn.reader_identity_type = 'character'
                        AND turn.reader_character_id = $4)
              )
              AND message.chapter_context IS NOT NULL
              AND message.chapter_context <= $5
            ORDER BY message.created_at DESC,
                     message.turn_id DESC NULLS LAST,
                     CASE message.role
                         WHEN 'character' THEN 0
                         WHEN 'user' THEN 1
                         ELSE 2
                     END,
                     message.id DESC
            LIMIT $6 OFFSET $7
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(reader_character_id)
        .bind(max_chapter)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(ChatMessage::from).collect())
    }
}
