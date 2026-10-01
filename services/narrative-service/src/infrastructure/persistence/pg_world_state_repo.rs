use std::collections::BTreeMap;

use crate::domain::entities::world_source::{
    WorldSourceCommand, WorldSourceDelta, WorldSourceError, WorldSourceOperation,
};
use anyhow::{ensure, Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::entities::{
    game_rules::GameRuleTemplate,
    narrative_node::WorldState,
    narrative_node::WorldStateError,
    player_entity::{PlayerEntity, RelationshipState},
    world_session::WorldEntryContext,
};
use crate::domain::repositories::{
    CharacterContextReadModel, CharacterContextSnapshotRepository, WorldStateRepository,
};

use super::{
    ensure_choice_projection_consistent, pg_narrative_repo::UserChoiceRow,
    pg_world_turn_repo::JournalRow,
};

const CHARACTER_CONTEXT_SNAPSHOT_TRANSACTION: &str =
    "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY";

#[derive(Debug, FromRow)]
struct WorldStateRow {
    user_id: Uuid,
    novel_id: Uuid,
    state: serde_json::Value,
    updated_at: DateTime<Utc>,
}

impl From<WorldStateRow> for WorldState {
    fn from(r: WorldStateRow) -> Self {
        WorldState {
            user_id: r.user_id,
            novel_id: r.novel_id,
            state: r.state,
            updated_at: r.updated_at,
        }
    }
}

#[derive(FromRow)]
struct SourceOperationRow {
    id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    expected_turn_number: i64,
    previous_source_chapter: i32,
    source_chapter: i32,
    source_context: serde_json::Value,
    request_fingerprint: Vec<u8>,
}

impl TryFrom<SourceOperationRow> for WorldSourceOperation {
    type Error = anyhow::Error;
    fn try_from(row: SourceOperationRow) -> Result<Self> {
        let operation = Self {
            operation_id: row.id,
            user_id: row.user_id,
            novel_id: row.novel_id,
            command: WorldSourceCommand {
                expected_turn_number: row.expected_turn_number,
                expected_source_chapter: row.previous_source_chapter,
                target_chapter: row.source_chapter,
            },
            source_context: serde_json::from_value(row.source_context)?,
        };
        operation.command.validate()?;
        let expected: [u8; 32] = Sha256::digest(serde_json::to_vec(&operation.command)?).into();
        ensure!(
            row.request_fingerprint == expected,
            "source operation identity is inconsistent"
        );
        operation.source_context.validate_source()?;
        ensure!(
            operation.source_context.unlocked_through_chapter == operation.command.target_chapter
        );
        Ok(operation)
    }
}

pub struct PgWorldStateRepository {
    pool: PgPool,
}

impl PgWorldStateRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn ensure_default_world_state(&self, user_id: Uuid, novel_id: Uuid) -> Result<()> {
        let world_state = WorldState::new(user_id, novel_id);
        sqlx::query(
            r#"
            INSERT INTO world_states (id, user_id, novel_id, state, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id, novel_id) DO NOTHING
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(world_state.user_id)
        .bind(world_state.novel_id)
        .bind(&world_state.state)
        .bind(world_state.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl WorldStateRepository for PgWorldStateRepository {
    async fn get_or_create(&self, user_id: Uuid, novel_id: Uuid) -> Result<WorldState> {
        self.ensure_default_world_state(user_id, novel_id).await?;

        let row = sqlx::query_as::<_, WorldStateRow>(
            r#"
            SELECT user_id, novel_id, state, updated_at
            FROM world_states
            WHERE user_id = $1 AND novel_id = $2
            "#,
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&self.pool)
        .await?;
        let state = WorldState::from(row);
        state.player_entity()?;
        Ok(state)
    }

    async fn create_player_entity(&self, player: &PlayerEntity) -> Result<PlayerEntity> {
        player.validate()?;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query_as::<_, WorldStateRow>(
            r#"
            SELECT user_id, novel_id, state, updated_at
            FROM world_states
            WHERE user_id = $1 AND novel_id = $2
            FOR UPDATE
            "#,
        )
        .bind(player.user_id)
        .bind(player.novel_id)
        .fetch_one(&mut *transaction)
        .await?;
        let mut world_state = WorldState::from(row);
        ensure_choice_projection_consistent(&mut *transaction, player.user_id, player.novel_id)
            .await?;
        if let Some(existing) = world_state.player_entity()? {
            if existing.canonical_checkpoint_chapter != player.canonical_checkpoint_chapter
                || !existing.matches_definition(
                    &player.name,
                    &player.background,
                    &player.capabilities,
                    player.location_id.as_deref(),
                    &player.inventory,
                )
                || !existing.matches_rules(&player.initial_rules())
            {
                return Err(WorldStateError::TimelineConflict(
                    "PlayerEntity was concurrently created with a different checkpoint or definition"
                        .into(),
                )
                .into());
            }
            transaction.commit().await?;
            return Ok(existing);
        }
        world_state.validate_world_entry_checkpoint(player.canonical_checkpoint_chapter)?;
        let legacy_relationships = world_state
            .state
            .as_object()
            .context("world state root must be an object")?
            .get("relationships")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let mut stored = player.clone();
        stored.relationships =
            serde_json::from_value::<BTreeMap<Uuid, RelationshipState>>(legacy_relationships)
                .context("legacy relationships are invalid")?;
        stored.validate()?;
        let root = world_state
            .state
            .as_object_mut()
            .context("world state root must be an object")?;
        root.remove("relationships");
        root.insert("player_entity".into(), serde_json::to_value(&stored)?);
        world_state.updated_at = Utc::now();
        sqlx::query(
            r#"
            UPDATE world_states
            SET state = $3, updated_at = $4
            WHERE user_id = $1
              AND novel_id = $2
            "#,
        )
        .bind(player.user_id)
        .bind(player.novel_id)
        .bind(&world_state.state)
        .bind(world_state.updated_at)
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(stored)
    }

    async fn start_open_world(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        context: &WorldEntryContext,
        game_rules: Option<&GameRuleTemplate>,
    ) -> Result<WorldState> {
        context.validate()?;
        let mut transaction = self.pool.begin().await?;
        let row = sqlx::query_as::<_, WorldStateRow>(
            r#"
            SELECT user_id, novel_id, state, updated_at
            FROM world_states
            WHERE user_id = $1 AND novel_id = $2
            FOR UPDATE
            "#,
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&mut *transaction)
        .await?;
        let mut world_state = WorldState::from(row);
        ensure_choice_projection_consistent(&mut *transaction, user_id, novel_id).await?;
        let before = world_state.state.clone();
        world_state.start_open_world_with_rules(context, game_rules)?;
        if world_state.state != before {
            let row = sqlx::query_as::<_, WorldStateRow>(
                r#"
                UPDATE world_states
                SET state = $3, updated_at = $4
                WHERE user_id = $1 AND novel_id = $2
                RETURNING user_id, novel_id, state, updated_at
                "#,
            )
            .bind(user_id)
            .bind(novel_id)
            .bind(&world_state.state)
            .bind(world_state.updated_at)
            .fetch_one(&mut *transaction)
            .await?;
            world_state = WorldState::from(row);
        }
        transaction.commit().await?;
        Ok(world_state)
    }

    async fn find_source_operation(
        &self,
        operation_id: Uuid,
    ) -> Result<Option<WorldSourceOperation>> {
        sqlx::query_as::<_, SourceOperationRow>(
            "SELECT id,user_id,novel_id,expected_turn_number,previous_source_chapter,source_chapter,source_context,request_fingerprint \
             FROM world_source_operations WHERE id=$1",
        ).bind(operation_id).fetch_optional(&self.pool).await?.map(TryInto::try_into).transpose()
    }

    async fn extend_world_source(
        &self,
        operation_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        command: &WorldSourceCommand,
        delta: &WorldSourceDelta,
    ) -> Result<WorldSourceOperation> {
        // One bounded database attempt. An uncertain acknowledgement is replayed
        // with the same logical key; this adapter never retries a write.
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
        command.validate()?;
        ensure!(
            operation_id.get_version() == Some(uuid::Version::Random)
                && operation_id.get_variant() == uuid::Variant::RFC4122,
            "invalid source operation key"
        );
        let mut tx = self.pool.begin().await?;
        sqlx::query("SELECT pg_catalog.set_config('statement_timeout', '5s', true), pg_catalog.set_config('lock_timeout', '3s', true)").execute(&mut *tx).await?;
        let row = sqlx::query_as::<_, WorldStateRow>(
            "SELECT user_id,novel_id,state,updated_at FROM world_states \
             WHERE user_id=$1 AND novel_id=$2 FOR UPDATE",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&mut *tx)
        .await?;
        let old = sqlx::query_as::<_, SourceOperationRow>(
            "SELECT id,user_id,novel_id,expected_turn_number,previous_source_chapter,source_chapter,source_context,request_fingerprint \
             FROM world_source_operations WHERE id=$1 FOR UPDATE",
        ).bind(operation_id).fetch_optional(&mut *tx).await?;
        if let Some(old) = old {
            let old = WorldSourceOperation::try_from(old)?;
            if old.user_id != user_id || old.novel_id != novel_id || old.command != *command {
                return Err(WorldSourceError::KeyConflict.into());
            }
            return Ok(old);
        }
        ensure_choice_projection_consistent(&mut *tx, user_id, novel_id).await?;
        let busy: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM world_turns WHERE user_id=$1 AND novel_id=$2 \
             AND (status='in_progress' OR (status='completed' AND memory_projection_status='pending')))",
        ).bind(user_id).bind(novel_id).fetch_one(&mut *tx).await?;
        if busy {
            return Err(WorldSourceError::Busy.into());
        }
        let mut state = WorldState::from(row);
        state.extend_world_source(command, delta)?;
        let context = state
            .open_world()?
            .context("source world missing")?
            .context()
            .clone();
        let fingerprint: [u8; 32] = Sha256::digest(serde_json::to_vec(command)?).into();
        let inserted = sqlx::query(
            "INSERT INTO world_source_operations(id,user_id,novel_id,request_fingerprint, \
             expected_turn_number,previous_source_chapter,source_chapter,source_context) \
             VALUES($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(id) DO NOTHING",
        )
        .bind(operation_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(fingerprint.as_slice())
        .bind(command.expected_turn_number)
        .bind(command.expected_source_chapter)
        .bind(command.target_chapter)
        .bind(serde_json::to_value(&context)?)
        .execute(&mut *tx)
        .await?;
        if inserted.rows_affected() != 1 {
            return Err(WorldSourceError::KeyConflict.into());
        }
        sqlx::query(
            "UPDATE world_states SET state=$3,updated_at=$4 WHERE user_id=$1 AND novel_id=$2",
        )
        .bind(user_id)
        .bind(novel_id)
        .bind(&state.state)
        .bind(state.updated_at)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(WorldSourceOperation {
            operation_id,
            user_id,
            novel_id,
            command: command.clone(),
            source_context: context,
        })

        }).await.context("world authority transaction deadline")?
    }

    async fn update(&self, state: &WorldState) -> Result<()> {
        state.player_entity()?;
        sqlx::query(
            r#"
            UPDATE world_states
            SET state = $3, updated_at = $4
            WHERE user_id = $1 AND novel_id = $2
            "#,
        )
        .bind(state.user_id)
        .bind(state.novel_id)
        .bind(&state.state)
        .bind(state.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

#[async_trait]
impl CharacterContextSnapshotRepository for PgWorldStateRepository {
    async fn read_character_context_snapshot(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        journal_limit: usize,
    ) -> Result<CharacterContextReadModel> {
        ensure!(
            (1..=100).contains(&journal_limit),
            "journal limit must be 1-100"
        );

        // Finish the idempotent default write before establishing the read-only
        // MVCC snapshot. Every fact used to derive the revision and context is
        // then read from the same PostgreSQL snapshot.
        self.ensure_default_world_state(user_id, novel_id).await?;
        let mut transaction = self
            .pool
            .begin_with(CHARACTER_CONTEXT_SNAPSHOT_TRANSACTION)
            .await?;

        let world_state = sqlx::query_as::<_, WorldStateRow>(
            r#"
            SELECT user_id, novel_id, state, updated_at
            FROM world_states
            WHERE user_id = $1 AND novel_id = $2
            "#,
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_one(&mut *transaction)
        .await?
        .into();
        ensure_choice_projection_consistent(&mut *transaction, user_id, novel_id).await?;

        let choices = sqlx::query_as::<_, UserChoiceRow>(
            r#"
            SELECT id, user_id, novel_id, node_id, chapter_number,
                   choice_index, choice_text, consequence, transition, created_at
            FROM user_choices
            WHERE user_id = $1 AND novel_id = $2
            ORDER BY created_at ASC
            "#,
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>>>()?;

        let journal = sqlx::query_as::<_, JournalRow>(
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
        .bind(journal_limit as i64)
        .fetch_all(&mut *transaction)
        .await?
        .into_iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>>>()?;

        transaction.commit().await?;
        Ok(CharacterContextReadModel {
            world_state,
            choices,
            journal,
        })
    }
}

#[cfg(test)]
mod snapshot_contract_tests {
    use super::*;

    #[test]
    fn character_context_uses_a_repeatable_read_read_only_transaction() {
        assert_eq!(
            CHARACTER_CONTEXT_SNAPSHOT_TRANSACTION,
            "BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY"
        );
    }
}
