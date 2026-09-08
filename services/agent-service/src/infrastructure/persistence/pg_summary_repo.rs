use anyhow::{ensure, Result};
use async_trait::async_trait;
use sqlx::{FromRow, PgConnection};
use std::{future::Future, time::Duration};
use uuid::Uuid;

use super::pg_chat_repo::{ChatMessageRow, PgChatRepository};
use crate::domain::{
    entities::memory::{Memory, MemoryLayer},
    repositories::{
        valid_summary_output, SummaryOutcome, SummarySource, SummaryWindow, SummaryWindowRepository,
    },
};

#[derive(FromRow)]
struct WindowRow {
    id: Uuid,
    user_id: Uuid,
    character_id: Uuid,
    novel_id: Uuid,
    summary_sequence: i64,
    summary_memory_id: Uuid,
    summary_claim_attempt: i64,
}

impl From<WindowRow> for SummaryWindow {
    fn from(row: WindowRow) -> Self {
        Self {
            id: row.id,
            user_id: row.user_id,
            character_id: row.character_id,
            novel_id: row.novel_id,
            sequence: row.summary_sequence,
            memory_id: row.summary_memory_id,
            attempt: row.summary_claim_attempt,
        }
    }
}

#[derive(FromRow)]
struct SourceRow {
    summary_sequence: i64,
    source_turn_id: Uuid,
    source_chapter: i32,
    source_identity: Option<String>,
    #[sqlx(flatten)]
    message: ChatMessageRow,
}

async fn bounded<T>(work: impl Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(Duration::from_secs(3), work).await?
}

async fn load_sources(
    connection: &mut PgConnection,
    window: &SummaryWindow,
) -> Result<Vec<SummarySource>> {
    let rows = sqlx::query_as::<_, SourceRow>(
        r#"
        SELECT turn.summary_sequence, turn.id AS source_turn_id,
               turn.chapter_context AS source_chapter, turn.reader_identity AS source_identity,
               message.id, message.turn_id, message.user_id, message.character_id,
               message.novel_id, message.role, message.content, message.reader_identity,
               message.chapter_context, turn.persona_source_chapter_high_water, message.created_at
        FROM chat_turns AS turn JOIN chat_messages AS message ON message.turn_id = turn.id
        WHERE turn.user_id = $1 AND turn.novel_id = $2 AND turn.character_id = $3
          AND turn.summary_sequence BETWEEN $4 - 9 AND $4
          AND turn.status = 'completed' AND turn.reader_identity_type = 'self'
          AND turn.reader_character_id IS NULL
        ORDER BY turn.summary_sequence, CASE message.role WHEN 'user' THEN 0 ELSE 1 END
        LIMIT 21
        FOR SHARE OF turn, message
        "#,
    )
    .bind(window.user_id)
    .bind(window.novel_id)
    .bind(window.character_id)
    .bind(window.sequence)
    .fetch_all(connection)
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| SummarySource {
            sequence: row.summary_sequence,
            turn_id: row.source_turn_id,
            chapter: row.source_chapter,
            reader_identity: row.source_identity,
            message: row.message.into(),
        })
        .collect())
}

#[async_trait]
impl SummaryWindowRepository for PgChatRepository {
    async fn due_windows(&self) -> Result<Vec<SummaryWindow>> {
        bounded(async {
            let mut tx = self.pool.begin().await?;
            sqlx::query_as::<_, (String,)>("SELECT pg_catalog.set_config('statement_timeout', '3000', true)").fetch_one(&mut *tx).await?;
            // Expired dispatches never return to pending. The bounded sweep also
            // recovers claims that provably never crossed dispatch-start.
            sqlx::query(r#"
                WITH expired AS (
                    SELECT id, summary_state, summary_claim_attempt
                    FROM chat_turns WHERE summary_state IN ('claimed', 'dispatched')
                      AND summary_lease_expires_at <= clock_timestamp()
                    ORDER BY summary_lease_expires_at, id LIMIT 10 FOR UPDATE SKIP LOCKED
                )
                UPDATE chat_turns AS turn SET
                    summary_state = CASE expired.summary_state WHEN 'claimed' THEN 'pending' ELSE 'unknown' END,
                    summary_lease_expires_at = NULL,
                    summary_next_attempt_at = CASE expired.summary_state WHEN 'claimed' THEN clock_timestamp() ELSE NULL END,
                    summary_failure_code = CASE expired.summary_state WHEN 'dispatched' THEN 'dispatch_unknown' ELSE NULL END
                FROM expired WHERE turn.id = expired.id
                  AND turn.summary_state = expired.summary_state
                  AND turn.summary_claim_attempt = expired.summary_claim_attempt
                  AND turn.summary_lease_expires_at <= clock_timestamp()
            "#).execute(&mut *tx).await?;
            let rows = sqlx::query_as::<_, WindowRow>(r#"
                SELECT id, user_id, character_id, novel_id, summary_sequence,
                       summary_memory_id, summary_claim_attempt
                FROM chat_turns WHERE summary_state = 'pending'
                  AND summary_next_attempt_at <= clock_timestamp()
                ORDER BY summary_next_attempt_at, id LIMIT 10
            "#).fetch_all(&mut *tx).await?;
            tx.commit().await?;
            Ok(rows.into_iter().map(Into::into).collect())
        }).await
    }

    async fn claim_window(&self, window: &SummaryWindow) -> Result<Option<SummaryWindow>> {
        bounded(async {
            let row = sqlx::query_as::<_, WindowRow>(
                r#"
                UPDATE chat_turns SET summary_state = 'claimed',
                    summary_claim_attempt = summary_claim_attempt + 1,
                    summary_lease_expires_at = clock_timestamp() + INTERVAL '15 seconds',
                    summary_next_attempt_at = NULL
                WHERE id = $1 AND summary_state = 'pending' AND summary_claim_attempt = $2
                  AND summary_next_attempt_at <= clock_timestamp()
                RETURNING id, user_id, character_id, novel_id, summary_sequence,
                          summary_memory_id, summary_claim_attempt
            "#,
            )
            .bind(window.id)
            .bind(window.attempt)
            .fetch_optional(&self.pool)
            .await?;
            Ok(row.map(Into::into))
        })
        .await
    }

    async fn defer_window(&self, window: &SummaryWindow, claimed: bool) -> Result<bool> {
        bounded(async {
            let result = sqlx::query(r#"
                UPDATE chat_turns SET summary_state = 'pending', summary_lease_expires_at = NULL,
                    summary_next_attempt_at = clock_timestamp() + INTERVAL '30 seconds'
                WHERE id = $1 AND summary_claim_attempt = $2
                  AND (($3 AND summary_state = 'claimed' AND summary_lease_expires_at > clock_timestamp())
                       OR (NOT $3 AND summary_state = 'pending' AND summary_next_attempt_at <= clock_timestamp()))
            "#).bind(window.id).bind(window.attempt).bind(claimed).execute(&self.pool).await?;
            Ok(result.rows_affected() == 1)
        }).await
    }

    async fn summary_sources(&self, window: &SummaryWindow) -> Result<Vec<SummarySource>> {
        bounded(async {
            let mut tx = self.pool.begin().await?;
            sqlx::query_as::<_, (String,)>(
                "SELECT pg_catalog.set_config('statement_timeout', '3000', true)",
            )
            .fetch_one(&mut *tx)
            .await?;
            let rows = load_sources(&mut tx, window).await?;
            tx.commit().await?;
            Ok(rows)
        })
        .await
    }

    async fn start_summary_dispatch(&self, window: &SummaryWindow) -> Result<bool> {
        bounded(async {
            let result = sqlx::query(
                r#"
                UPDATE chat_turns SET summary_state = 'dispatched',
                    summary_lease_expires_at = clock_timestamp() + INTERVAL '330 seconds'
                WHERE id = $1 AND summary_state = 'claimed' AND summary_claim_attempt = $2
                  AND summary_lease_expires_at > clock_timestamp()
            "#,
            )
            .bind(window.id)
            .bind(window.attempt)
            .execute(&self.pool)
            .await?;
            Ok(result.rows_affected() == 1)
        })
        .await
    }

    async fn finish_summary(&self, window: &SummaryWindow, memory: &Memory) -> Result<bool> {
        bounded(async {
            let mut tx = self.pool.begin().await?;
            sqlx::query_as::<_, (String,)>("SELECT pg_catalog.set_config('statement_timeout', '3000', true)").fetch_one(&mut *tx).await?;
            let row = sqlx::query_as::<_, WindowRow>(r#"
                SELECT id, user_id, character_id, novel_id, summary_sequence,
                       summary_memory_id, summary_claim_attempt FROM chat_turns
                WHERE id = $1 AND summary_state = 'dispatched' AND summary_claim_attempt = $2
                  AND summary_lease_expires_at > clock_timestamp() FOR UPDATE
            "#).bind(window.id).bind(window.attempt).fetch_optional(&mut *tx).await?;
            let Some(row) = row else { return Ok(false); };
            ensure!(SummaryWindow::from(row) == *window, "summary claim scope mismatch");
            let sources = load_sources(&mut tx, window).await?;
            let (chapter, persona) = window.validate_sources(&sources)?;
            ensure!(memory.id == window.memory_id && memory.layer == MemoryLayer::Mid
                && memory.user_id == window.user_id && memory.novel_id == window.novel_id
                && memory.character_id == window.character_id && memory.importance == 6
                && memory.chapter_number == Some(chapter)
                && memory.persona_source_chapter_high_water == Some(persona)
                && memory.embedding.is_none() && valid_summary_output(&memory.content), "invalid summary result");
            // No UPSERT: collision must roll the entire fenced publication back.
            sqlx::query(r#"
                INSERT INTO character_memories (id, character_id, user_id, novel_id, layer,
                    content, importance, chapter_number, persona_source_chapter_high_water, created_at)
                VALUES ($1, $2, $3, $4, 'mid', $5, 6, $6, $7, $8)
            "#).bind(memory.id).bind(memory.character_id).bind(memory.user_id).bind(memory.novel_id)
                .bind(&memory.content).bind(chapter).bind(persona).bind(memory.created_at)
                .execute(&mut *tx).await?;
            let result = sqlx::query(r#"
                UPDATE chat_turns SET summary_state = 'saved', summary_lease_expires_at = NULL
                WHERE id = $1 AND summary_state = 'dispatched' AND summary_claim_attempt = $2
                  AND summary_lease_expires_at > clock_timestamp()
            "#).bind(window.id).bind(window.attempt).execute(&mut *tx).await?;
            ensure!(result.rows_affected() == 1, "summary claim expired before publication");
            tx.commit().await?;
            Ok(true)
        }).await
    }

    async fn fail_summary(&self, window: &SummaryWindow, outcome: SummaryOutcome) -> Result<bool> {
        bounded(async {
            let (expected, terminal) = match outcome {
                SummaryOutcome::SourceInvalid => ("claimed", "failed"),
                SummaryOutcome::DispatchUnknown => ("dispatched", "unknown"),
                SummaryOutcome::OutputInvalid | SummaryOutcome::EligibilityChanged => {
                    ("dispatched", "failed")
                }
            };
            let result = sqlx::query(
                r#"
                UPDATE chat_turns SET summary_state = $3, summary_failure_code = $4,
                    summary_lease_expires_at = NULL, summary_next_attempt_at = NULL
                WHERE id = $1 AND summary_claim_attempt = $2 AND summary_state = $5
                  AND summary_lease_expires_at > clock_timestamp()
            "#,
            )
            .bind(window.id)
            .bind(window.attempt)
            .bind(terminal)
            .bind(outcome.to_str())
            .bind(expected)
            .execute(&self.pool)
            .await?;
            Ok(result.rows_affected() == 1)
        })
        .await
    }
}
