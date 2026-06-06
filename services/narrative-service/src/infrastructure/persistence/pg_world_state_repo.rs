use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::narrative_node::WorldState;
use crate::domain::repositories::WorldStateRepository;

#[derive(Debug, FromRow)]
struct WorldStateRow {
    id: Uuid,
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
        // Try to find existing world state
        let row = sqlx::query_as::<_, WorldStateRow>(
            "SELECT * FROM world_states WHERE user_id = $1 AND novel_id = $2",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row {
            return Ok(WorldState::from(row));
        }

        // Create default world state
        let ws = WorldState::new(user_id, novel_id);
        let id = Uuid::new_v4();

        sqlx::query(
            r#"
            INSERT INTO world_states (id, user_id, novel_id, state, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(id)
        .bind(ws.user_id)
        .bind(ws.novel_id)
        .bind(&ws.state)
        .bind(ws.updated_at)
        .execute(&self.pool)
        .await?;

        Ok(ws)
    }

    async fn update(&self, state: &WorldState) -> Result<()> {
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
