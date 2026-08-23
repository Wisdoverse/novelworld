use std::collections::BTreeMap;

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::entities::{
    game_rules::GameRuleTemplate,
    narrative_node::WorldState,
    player_entity::{PlayerEntity, RelationshipState},
    world_session::WorldEntryContext,
};
use crate::domain::repositories::WorldStateRepository;

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

pub struct PgWorldStateRepository {
    pool: PgPool,
}

impl PgWorldStateRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl WorldStateRepository for PgWorldStateRepository {
    async fn get_or_create(&self, user_id: Uuid, novel_id: Uuid) -> Result<WorldState> {
        let ws = WorldState::new(user_id, novel_id);
        sqlx::query(
            r#"
            INSERT INTO world_states (id, user_id, novel_id, state, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id, novel_id) DO NOTHING
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(ws.user_id)
        .bind(ws.novel_id)
        .bind(&ws.state)
        .bind(ws.updated_at)
        .execute(&self.pool)
        .await?;

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
        if let Some(existing) = world_state.player_entity()? {
            return Ok(existing);
        }
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
