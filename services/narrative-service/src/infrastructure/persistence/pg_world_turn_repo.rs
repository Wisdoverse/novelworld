use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{prelude::FromRow, PgPool};
use uuid::Uuid;

use crate::domain::entities::game_rules::{ActionCheck, AdjudicationDecision};
use crate::domain::{
    entities::{narrative_node::WorldState, world_session::WorldEntryContext},
    repositories::{
        BeginWorldTurn, MemoryProjectionStatus, RecoverableWorldTurn, WorldTurnClaim,
        WorldTurnJournalEntry, WorldTurnRepository, WorldTurnResult,
    },
};

use super::ensure_choice_projection_consistent;

#[derive(Debug, FromRow)]
struct WorldTurnRow {
    id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    request_fingerprint: Vec<u8>,
    action: serde_json::Value,
    resolution: Option<serde_json::Value>,
    expected_turn_number: i64,
    expected_source_chapter: Option<i32>,
    status: String,
    attempt: i64,
    failure_code: Option<String>,
    memory_projection_status: String,
    lease_expired: bool,
    result: Option<serde_json::Value>,
}

impl WorldTurnRow {
    fn action(&self) -> Result<crate::domain::entities::world_session::WorldAction> {
        serde_json::from_value(self.action.clone()).context("persisted world action is invalid")
    }

    fn matches(&self, claim: &WorldTurnClaim) -> Result<bool> {
        Ok(self.id == claim.id
            && self.user_id == claim.user_id
            && self.novel_id == claim.novel_id
            && self.request_fingerprint == claim.request_fingerprint
            && self.action()? == claim.action
            && self.expected_turn_number == claim.expected_turn_number
            && self.expected_source_chapter == claim.expected_source_chapter)
    }

    fn resolution(&self) -> Result<Option<crate::domain::entities::game_rules::ActionCheck>> {
        self.resolution
            .clone()
            .map(|value| serde_json::from_value(value).context("persisted action check is invalid"))
            .transpose()
    }

    fn claim(&self) -> Result<WorldTurnClaim> {
        let claim = WorldTurnClaim {
            id: self.id,
            user_id: self.user_id,
            novel_id: self.novel_id,
            request_fingerprint: self.request_fingerprint.clone(),
            action: self.action()?,
            resolution: self.resolution()?,
            expected_turn_number: self.expected_turn_number,
            expected_source_chapter: self.expected_source_chapter,
        };
        validate_claim(&claim)?;
        Ok(claim)
    }
}

#[derive(Debug, FromRow)]
struct WorldStateRow {
    user_id: Uuid,
    novel_id: Uuid,
    state: serde_json::Value,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, FromRow)]
struct RecoverableTurnRow {
    turn_id: Uuid,
    action: serde_json::Value,
    expected_turn_number: i64,
    expected_source_chapter: Option<i32>,
}

impl TryFrom<RecoverableTurnRow> for RecoverableWorldTurn {
    type Error = anyhow::Error;

    fn try_from(row: RecoverableTurnRow) -> Result<Self> {
        ensure!(!row.turn_id.is_nil() && row.expected_turn_number >= 0);
        Ok(Self {
            turn_id: row.turn_id,
            action: serde_json::from_value(row.action)
                .context("recoverable world action is invalid")?,
            expected_turn_number: row.expected_turn_number,
            expected_source_chapter: row.expected_source_chapter,
        })
    }
}

impl From<WorldStateRow> for WorldState {
    fn from(row: WorldStateRow) -> Self {
        Self {
            user_id: row.user_id,
            novel_id: row.novel_id,
            state: row.state,
            updated_at: row.updated_at,
        }
    }
}

#[derive(Debug, FromRow)]
pub(super) struct JournalRow {
    id: Uuid,
    turn_number: i64,
    expected_source_chapter: Option<i32>,
    memory_projection_status: String,
    action: serde_json::Value,
    resolution: Option<serde_json::Value>,
    transition: serde_json::Value,
    created_at: DateTime<Utc>,
    completed_at: DateTime<Utc>,
}

impl TryFrom<JournalRow> for WorldTurnJournalEntry {
    type Error = anyhow::Error;

    fn try_from(row: JournalRow) -> Result<Self> {
        Ok(Self {
            turn_id: row.id,
            turn_number: row.turn_number,
            expected_source_chapter: row.expected_source_chapter,
            memory_projection_status: MemoryProjectionStatus::from_str(
                &row.memory_projection_status,
            )
            .context("journal contains invalid memory projection status")?,
            action: serde_json::from_value(row.action)?,
            resolution: row.resolution.map(serde_json::from_value).transpose()?,
            transition: serde_json::from_value(row.transition)?,
            created_at: row.created_at,
            completed_at: row.completed_at,
        })
    }
}

pub struct PgWorldTurnRepository {
    pool: PgPool,
}

impl PgWorldTurnRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    fn completed_result(row: &WorldTurnRow) -> Result<WorldTurnResult> {
        let result = serde_json::from_value::<WorldTurnResult>(
            row.result
                .clone()
                .context("completed world turn is missing its result")?,
        )
        .context("completed world turn result is invalid")?;
        ensure!(result.turn_id == row.id);
        ensure!(result.action == row.action()?);
        ensure!(result.resolution == row.resolution()?);
        if let Some(check) = &result.resolution {
            check.validate_resolved()?;
        }
        ensure!(result.world_state.user_id == row.user_id);
        ensure!(result.world_state.novel_id == row.novel_id);
        let session = result
            .world_state
            .open_world()?
            .context("completed world turn is missing its world session")?;
        ensure!(session.turn_number == row.expected_turn_number + 1);
        Ok(result)
    }
}

#[async_trait]
impl WorldTurnRepository for PgWorldTurnRepository {
    async fn begin_turn(&self, claim: &WorldTurnClaim) -> Result<BeginWorldTurn> {
        // One bounded database attempt. An uncertain acknowledgement is replayed
        // with the same logical key; this adapter never retries a write.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
        validate_claim(claim)?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('statement_timeout', '5s', true), pg_catalog.set_config('lock_timeout', '3s', true)").execute(&mut *tx).await?;
        // All scope writers lock authority before journal, including completion
        // and source admission. Supersession is part of this same transaction.
        let state = sqlx::query_as::<_, WorldStateRow>(
            "SELECT user_id, novel_id, state, updated_at FROM world_states \
             WHERE user_id=$1 AND novel_id=$2 FOR UPDATE",
        )
        .bind(claim.user_id)
        .bind(claim.novel_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(state) = state else {
            return Ok(BeginWorldTurn::Stale);
        };
        ensure_choice_projection_consistent(&mut *tx, claim.user_id, claim.novel_id).await?;
        let row = sqlx::query_as::<_, WorldTurnRow>(
            "SELECT id,user_id,novel_id,request_fingerprint,action,resolution, \
             expected_turn_number,expected_source_chapter,status,attempt,failure_code, \
             memory_projection_status,COALESCE(lease_expires_at <= NOW(),FALSE) AS lease_expired,result \
             FROM world_turns WHERE id=$1 FOR UPDATE",
        ).bind(claim.id).fetch_optional(&mut *tx).await?;
        if let Some(row) = &row {
            if !row.matches(claim)? {
                return Ok(BeginWorldTurn::Conflict);
            }
            if row.status == "completed" {
                return Ok(BeginWorldTurn::Completed {
                    result: Box::new(Self::completed_result(row)?),
                    memory_projection: MemoryProjectionStatus::from_str(
                        &row.memory_projection_status,
                    )
                    .context("invalid completed memory projection")?,
                });
            }
            if row.failure_code.as_deref() == Some("superseded") {
                return Ok(BeginWorldTurn::Conflict);
            }
        }
        let state = WorldState::from(state);
        let session = state
            .open_world()?
            .context("world session has not started")?;
        if session.turn_number != claim.expected_turn_number
            || claim
                .expected_source_chapter
                .map_or(session.source_context.is_some(), |chapter| {
                    chapter != session.context().unlocked_through_chapter
                })
        {
            return Ok(BeginWorldTurn::Stale);
        }
        let busy: Option<(i64,)> = sqlx::query_as(
            "SELECT GREATEST(1,CEIL(EXTRACT(EPOCH FROM lease_expires_at-NOW()))::BIGINT) \
             FROM world_turns WHERE user_id=$1 AND novel_id=$2 \
             AND status='in_progress' AND lease_expires_at > NOW() LIMIT 1 FOR UPDATE",
        )
        .bind(claim.user_id)
        .bind(claim.novel_id)
        .fetch_optional(&mut *tx)
        .await?;
        if let Some((seconds,)) = busy {
            return Ok(BeginWorldTurn::InProgress {
                retry_after_seconds: seconds as u64,
            });
        }
        let pending: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM world_turns WHERE user_id=$1 AND novel_id=$2 \
             AND status='completed' AND memory_projection_status='pending')",
        )
        .bind(claim.user_id)
        .bind(claim.novel_id)
        .fetch_one(&mut *tx)
        .await?;
        if pending {
            return Ok(BeginWorldTurn::InProgress {
                retry_after_seconds: 30,
            });
        }
        sqlx::query(
            "UPDATE world_turns SET status='failed',lease_expires_at=NULL, \
             failure_code='superseded',updated_at=NOW() WHERE user_id=$1 AND novel_id=$2 \
             AND id<>$3 AND status='in_progress' AND lease_expires_at<=NOW()",
        )
        .bind(claim.user_id)
        .bind(claim.novel_id)
        .bind(claim.id)
        .execute(&mut *tx)
        .await?;
        let (stored, attempt) = if let Some(row) = row {
            ensure!(
                row.status == "failed" || row.status == "in_progress",
                "unknown world turn status"
            );
            let attempt: i64 = sqlx::query_scalar(
                "UPDATE world_turns SET status='in_progress',attempt=attempt+1, \
                 lease_expires_at=NOW()+INTERVAL '2 minutes',failure_code=NULL,updated_at=NOW() \
                 WHERE id=$1 RETURNING attempt",
            )
            .bind(claim.id)
            .fetch_one(&mut *tx)
            .await?;
            (row.claim()?, attempt)
        } else {
            let inserted = sqlx::query(
                "INSERT INTO world_turns(id,user_id,novel_id,request_fingerprint,action,resolution, \
                 expected_turn_number,expected_source_chapter,status,attempt,lease_expires_at) \
                 VALUES($1,$2,$3,$4,$5,$6,$7,$8,'in_progress',1,NOW()+INTERVAL '2 minutes') ON CONFLICT(id) DO NOTHING",
            ).bind(claim.id).bind(claim.user_id).bind(claim.novel_id).bind(&claim.request_fingerprint)
             .bind(serde_json::to_value(&claim.action)?)
             .bind(claim.resolution.as_ref().map(serde_json::to_value).transpose()?)
             .bind(claim.expected_turn_number).bind(claim.expected_source_chapter)
             .execute(&mut *tx).await?;
            if inserted.rows_affected() != 1 {
                return Ok(BeginWorldTurn::Conflict);
            }
            (claim.clone(), 1)
        };
        tx.commit().await?;
        Ok(BeginWorldTurn::Acquired {
            claim: Box::new(stored),
            attempt,
        })

        }).await.context("world authority transaction deadline")?
    }

    async fn settle_adjudication(
        &self,
        claim: &WorldTurnClaim,
        attempt: i64,
        decision: AdjudicationDecision,
    ) -> Result<Option<ActionCheck>> {
        validate_claim(claim)?;
        let previous = claim
            .resolution
            .as_ref()
            .context("action check is missing")?;
        let resolved = previous.settle_adjudication(decision)?;
        let result = sqlx::query(
            r#"
            UPDATE world_turns
            SET resolution = $1, updated_at = NOW()
            WHERE id = $2 AND user_id = $3 AND novel_id = $4
              AND expected_turn_number = $5 AND request_fingerprint = $6
              AND action = $7 AND resolution = $8 AND attempt = $9
              AND status = 'in_progress' AND lease_expires_at > NOW()
            "#,
        )
        .bind(serde_json::to_value(&resolved)?)
        .bind(claim.id)
        .bind(claim.user_id)
        .bind(claim.novel_id)
        .bind(claim.expected_turn_number)
        .bind(&claim.request_fingerprint)
        .bind(serde_json::to_value(&claim.action)?)
        .bind(serde_json::to_value(previous)?)
        .bind(attempt)
        .execute(&self.pool)
        .await?;
        Ok((result.rows_affected() == 1).then_some(resolved))
    }

    async fn recoverable_turn(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<Option<RecoverableWorldTurn>> {
        sqlx::query_as::<_, RecoverableTurnRow>(
            r#"
            SELECT id AS turn_id, action, expected_turn_number, expected_source_chapter
            FROM world_turns
            WHERE user_id = $1 AND novel_id = $2 AND status = 'in_progress'
            "#,
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_optional(&self.pool)
        .await?
        .map(TryInto::try_into)
        .transpose()
    }

    async fn rotate_pending_memory_projections(
        &self,
        limit: usize,
    ) -> Result<Vec<WorldTurnResult>> {
        ensure!(
            (1..=100).contains(&limit),
            "memory projection recovery limit must be 1-100"
        );
        let rows = sqlx::query_as::<_, WorldTurnRow>(
            r#"
            WITH candidates AS (
                SELECT id, updated_at AS scan_position
                FROM world_turns
                WHERE status = 'completed' AND memory_projection_status = 'pending'
                ORDER BY updated_at ASC, id ASC
                LIMIT $1
                FOR UPDATE SKIP LOCKED
            ), rotated AS (
                UPDATE world_turns AS turn
                SET updated_at = NOW()
                FROM candidates
                WHERE turn.id = candidates.id
                RETURNING turn.id, turn.user_id, turn.novel_id,
                          turn.request_fingerprint, turn.action, turn.resolution,
                          turn.expected_turn_number, turn.expected_source_chapter, turn.status, turn.attempt,
                          turn.failure_code, turn.memory_projection_status,
                          turn.result, candidates.scan_position
            )
            SELECT id, user_id, novel_id, request_fingerprint, action, resolution,
                   expected_turn_number, expected_source_chapter, status, attempt, failure_code,
                   memory_projection_status, FALSE AS lease_expired, result
            FROM rotated
            ORDER BY scan_position ASC, id ASC
            "#,
        )
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        let mut results = Vec::with_capacity(rows.len());
        for row in rows {
            match Self::completed_result(&row) {
                Ok(result) => results.push(result),
                Err(error) => tracing::error!(
                    turn_id = %row.id,
                    error = ?error,
                    "pending world turn has an invalid committed result"
                ),
            }
        }
        Ok(results)
    }

    async fn renew_turn(&self, turn_id: Uuid, attempt: i64) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE world_turns
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
        claim: &WorldTurnClaim,
        attempt: i64,
        transition: &crate::domain::entities::world_session::WorldTurnTransition,
        context: &WorldEntryContext,
    ) -> Result<WorldTurnResult> {
        // One bounded database attempt. An uncertain acknowledgement is replayed
        // with the same logical key; this adapter never retries a write.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            validate_claim(claim)?;
            if let Some(check) = &claim.resolution {
                check.validate_resolved()?;
            }
            context.validate_source()?;
            let mut transaction = self.pool.begin().await?;
            sqlx::query("SELECT pg_catalog.set_config('statement_timeout', '5s', true), pg_catalog.set_config('lock_timeout', '3s', true)")
                .execute(&mut *transaction)
                .await?;

            let state_row = sqlx::query_as::<_, WorldStateRow>(
                r#"
            SELECT user_id, novel_id, state, updated_at
            FROM world_states
            WHERE user_id = $1 AND novel_id = $2
            FOR UPDATE
            "#,
            )
            .bind(claim.user_id)
            .bind(claim.novel_id)
            .fetch_one(&mut *transaction)
            .await?;
            let turn = sqlx::query_as::<_, WorldTurnRow>(
                r#"
            SELECT id, user_id, novel_id, request_fingerprint, action, resolution,
                   expected_turn_number, expected_source_chapter, status, attempt, failure_code,
                   memory_projection_status,
                   COALESCE(lease_expires_at <= NOW(), FALSE) AS lease_expired,
                   result
            FROM world_turns
            WHERE id = $1
            FOR UPDATE
            "#,
            )
            .bind(claim.id)
            .fetch_one(&mut *transaction)
            .await?;
            ensure!(turn.matches(claim)?, "world turn claim conflicts");
            ensure!(
                turn.resolution()? == claim.resolution,
                "action check is not the frozen resolution"
            );
            ensure!(turn.status == "in_progress" && turn.attempt == attempt && !turn.lease_expired);

            let mut world_state = WorldState::from(state_row);
            ensure_choice_projection_consistent(&mut *transaction, claim.user_id, claim.novel_id)
                .await?;
            let session = world_state
                .open_world()?
                .context("world session has not started")?;
            ensure!(
                session.turn_number == claim.expected_turn_number,
                "stale world turn"
            );
            ensure!(
                claim
                    .expected_source_chapter
                    .map_or(session.source_context.is_none(), |chapter| chapter
                        == session.context().unlocked_through_chapter),
                "stale world source"
            );
            ensure!(session.context() == context, "world entry context changed");
            world_state.apply_world_turn_with_check(
                claim.id,
                &claim.action,
                transition,
                context,
                claim.resolution.as_ref(),
            )?;

            world_state.updated_at = sqlx::query_scalar(
                r#"
            UPDATE world_states
            SET state = $3, updated_at = $4
            WHERE user_id = $1 AND novel_id = $2
            RETURNING updated_at
            "#,
            )
            .bind(claim.user_id)
            .bind(claim.novel_id)
            .bind(&world_state.state)
            .bind(world_state.updated_at)
            .fetch_one(&mut *transaction)
            .await?;
            let result = WorldTurnResult {
                turn_id: claim.id,
                action: claim.action.clone(),
                resolution: claim.resolution.clone(),
                transition: transition.clone(),
                world_state,
            };
            let transition_json = serde_json::to_value(transition)?;
            let result_json = serde_json::to_value(&result)?;

            let completed = sqlx::query(
                r#"
            UPDATE world_turns
            SET status = 'completed', lease_expires_at = NULL,
                transition = $3, result = $4, failure_code = NULL,
                completed_at = NOW(), updated_at = NOW()
            WHERE id = $1 AND attempt = $2 AND status = 'in_progress'
            "#,
            )
            .bind(claim.id)
            .bind(attempt)
            .bind(transition_json)
            .bind(result_json)
            .execute(&mut *transaction)
            .await?;
            ensure!(
                completed.rows_affected() == 1,
                "world turn claim was fenced"
            );
            transaction.commit().await?;
            Ok(result)
        })
        .await
        .context("world authority transaction deadline")?
    }

    async fn fail_turn(&self, turn_id: Uuid, attempt: i64, failure_code: &str) -> Result<bool> {
        ensure!(
            !failure_code.is_empty()
                && failure_code.len() <= 64
                && failure_code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-')),
            "invalid world turn failure code"
        );
        let result = sqlx::query(
            r#"
            UPDATE world_turns
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

    async fn finish_memory_projection(
        &self,
        turn_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        status: MemoryProjectionStatus,
    ) -> Result<bool> {
        ensure!(
            status.is_terminal(),
            "memory projection status must be terminal"
        );
        let value = status.to_str();
        let result = sqlx::query(
            r#"
            UPDATE world_turns
            SET memory_projection_status = $4,
                memory_projection_completed_at = COALESCE(
                    memory_projection_completed_at,
                    NOW()
                ),
                updated_at = NOW()
            WHERE id = $1 AND user_id = $2 AND novel_id = $3
              AND status = 'completed'
              AND memory_projection_status IN ('pending', $4)
            "#,
        )
        .bind(turn_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(value)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn journal(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        limit: usize,
    ) -> Result<Vec<WorldTurnJournalEntry>> {
        ensure!((1..=100).contains(&limit), "journal limit must be 1-100");
        let rows = sqlx::query_as::<_, JournalRow>(
            r#"
            SELECT id, turn_number, expected_source_chapter, memory_projection_status,
                   action, resolution, transition, created_at, completed_at
            FROM (
                SELECT id, expected_turn_number + 1 AS turn_number, expected_source_chapter, action, resolution,
                       transition, memory_projection_status, created_at, completed_at
                FROM world_turns
                WHERE user_id = $1 AND novel_id = $2 AND status = 'completed'
                ORDER BY expected_turn_number DESC
                LIMIT $3
            ) AS recent
            ORDER BY turn_number ASC
            "#,
        )
        .bind(user_id)
        .bind(novel_id)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter().map(TryInto::try_into).collect()
    }
}

fn validate_claim(claim: &WorldTurnClaim) -> Result<()> {
    ensure!(!claim.id.is_nil() && !claim.user_id.is_nil() && !claim.novel_id.is_nil());
    ensure!(claim.request_fingerprint.len() == 32);
    ensure!(claim
        .expected_source_chapter
        .is_none_or(|chapter| chapter >= 1));
    ensure!(claim.expected_turn_number >= 0);
    if let Some(resolution) = &claim.resolution {
        resolution.validate()?;
    }
    Ok(())
}
