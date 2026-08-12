use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::entities::narrative_node::{NarrativeChoice, NarrativeNode, WorldState};
use crate::domain::repositories::{
    ChoiceCommit, ChoiceCommitResult, NarrativeNodeRepository, UserChoiceRecord,
    UserChoiceRepository,
};

// ─── NarrativeNode persistence ──────────────────────────────────────────────

#[derive(Debug, FromRow)]
struct NarrativeNodeRow {
    id: Uuid,
    novel_id: Uuid,
    chapter_number: i32,
    description: String,
    anchor_quote: Option<String>,
    choices: serde_json::Value,
    created_at: DateTime<Utc>,
}

impl From<NarrativeNodeRow> for NarrativeNode {
    fn from(r: NarrativeNodeRow) -> Self {
        let choices: Vec<NarrativeChoice> = serde_json::from_value(r.choices).unwrap_or_default();
        NarrativeNode {
            id: r.id,
            novel_id: r.novel_id,
            chapter_number: r.chapter_number,
            description: r.description,
            anchor_quote: r.anchor_quote,
            choices,
            created_at: r.created_at,
        }
    }
}

pub struct PgNarrativeNodeRepository {
    pool: PgPool,
}

impl PgNarrativeNodeRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl NarrativeNodeRepository for PgNarrativeNodeRepository {
    async fn save(&self, node: &NarrativeNode) -> Result<()> {
        let choices_json = serde_json::to_value(&node.choices)?;
        sqlx::query(
            r#"
            INSERT INTO narrative_nodes (
                id, novel_id, chapter_number, description, anchor_quote, choices, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (novel_id, chapter_number) DO UPDATE SET
                description = EXCLUDED.description,
                anchor_quote = EXCLUDED.anchor_quote,
                choices = EXCLUDED.choices
            "#,
        )
        .bind(node.id)
        .bind(node.novel_id)
        .bind(node.chapter_number)
        .bind(&node.description)
        .bind(&node.anchor_quote)
        .bind(choices_json)
        .bind(node.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_by_chapter(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
    ) -> Result<Option<NarrativeNode>> {
        let row = sqlx::query_as::<_, NarrativeNodeRow>(
            "SELECT * FROM narrative_nodes WHERE novel_id = $1 AND chapter_number = $2",
        )
        .bind(novel_id)
        .bind(chapter_number)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(NarrativeNode::from))
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<NarrativeNode>> {
        let row =
            sqlx::query_as::<_, NarrativeNodeRow>("SELECT * FROM narrative_nodes WHERE id = $1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;

        Ok(row.map(NarrativeNode::from))
    }
}

// ─── UserChoice persistence ─────────────────────────────────────────────────

#[derive(Debug, FromRow)]
struct UserChoiceRow {
    id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    node_id: Uuid,
    chapter_number: i32,
    choice_index: i32,
    choice_text: String,
    consequence: Option<String>,
    created_at: DateTime<Utc>,
}

impl From<UserChoiceRow> for UserChoiceRecord {
    fn from(r: UserChoiceRow) -> Self {
        UserChoiceRecord {
            id: r.id,
            user_id: r.user_id,
            novel_id: r.novel_id,
            node_id: r.node_id,
            chapter_number: r.chapter_number,
            choice_index: r.choice_index,
            choice_text: r.choice_text,
            consequence: r.consequence,
            created_at: r.created_at,
        }
    }
}

pub struct PgUserChoiceRepository {
    pool: PgPool,
}

impl PgUserChoiceRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserChoiceRepository for PgUserChoiceRepository {
    async fn commit_choice(&self, draft: &ChoiceCommit) -> Result<ChoiceCommitResult> {
        let mut transaction = self.pool.begin().await?;
        let initial_state = WorldState::new(draft.user_id, draft.novel_id);
        sqlx::query(
            r#"
            INSERT INTO world_states (id, user_id, novel_id, state, updated_at)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (user_id, novel_id) DO NOTHING
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(draft.user_id)
        .bind(draft.novel_id)
        .bind(&initial_state.state)
        .bind(initial_state.updated_at)
        .execute(&mut *transaction)
        .await?;

        let state_row = sqlx::query_as::<_, ChoiceWorldStateRow>(
            r#"
            SELECT user_id, novel_id, state, updated_at
            FROM world_states
            WHERE user_id = $1 AND novel_id = $2
            FOR UPDATE
            "#,
        )
        .bind(draft.user_id)
        .bind(draft.novel_id)
        .fetch_one(&mut *transaction)
        .await?;
        let mut world_state = WorldState::from(state_row);

        let inserted = sqlx::query_as::<_, UserChoiceRow>(
            r#"
            INSERT INTO user_choices (
                id, user_id, novel_id, node_id, chapter_number,
                choice_index, choice_text, consequence, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            ON CONFLICT (user_id, node_id) DO NOTHING
            RETURNING id, user_id, novel_id, node_id, chapter_number,
                      choice_index, choice_text, consequence, created_at
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(draft.user_id)
        .bind(draft.novel_id)
        .bind(draft.node_id)
        .bind(draft.chapter_number)
        .bind(draft.choice_index)
        .bind(&draft.choice_text)
        .bind(&draft.consequence)
        .bind(Utc::now())
        .fetch_optional(&mut *transaction)
        .await?;

        let mut choice = match inserted {
            Some(row) => UserChoiceRecord::from(row),
            None => sqlx::query_as::<_, UserChoiceRow>(
                r#"
                SELECT id, user_id, novel_id, node_id, chapter_number,
                       choice_index, choice_text, consequence, created_at
                FROM user_choices
                WHERE user_id = $1 AND node_id = $2
                FOR UPDATE
                "#,
            )
            .bind(draft.user_id)
            .bind(draft.node_id)
            .fetch_one(&mut *transaction)
            .await?
            .into(),
        };

        if choice.consequence.as_deref().is_none_or(str::is_empty) {
            sqlx::query(
                "UPDATE user_choices SET consequence = $3 WHERE user_id = $1 AND node_id = $2",
            )
            .bind(choice.user_id)
            .bind(choice.node_id)
            .bind(&draft.consequence)
            .execute(&mut *transaction)
            .await?;
            choice.consequence = Some(draft.consequence.clone());
        }
        let consequence = choice.consequence.as_deref().unwrap_or_default();
        if world_state.record_choice(
            choice.node_id,
            choice.chapter_number,
            choice.choice_index,
            &choice.choice_text,
            consequence,
        )? {
            sqlx::query(
                "UPDATE world_states SET state = $3, updated_at = $4 WHERE user_id = $1 AND novel_id = $2",
            )
            .bind(world_state.user_id)
            .bind(world_state.novel_id)
            .bind(&world_state.state)
            .bind(world_state.updated_at)
            .execute(&mut *transaction)
            .await?;
        }

        transaction.commit().await?;
        Ok(ChoiceCommitResult {
            choice,
            world_state,
        })
    }

    async fn find_user_choice(
        &self,
        user_id: Uuid,
        node_id: Uuid,
    ) -> Result<Option<UserChoiceRecord>> {
        let row = sqlx::query_as::<_, UserChoiceRow>(
            "SELECT * FROM user_choices WHERE user_id = $1 AND node_id = $2",
        )
        .bind(user_id)
        .bind(node_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(UserChoiceRecord::from))
    }

    async fn find_by_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<Vec<UserChoiceRecord>> {
        let rows = sqlx::query_as::<_, UserChoiceRow>(
            "SELECT * FROM user_choices WHERE user_id = $1 AND novel_id = $2 ORDER BY created_at ASC",
        )
        .bind(user_id)
        .bind(novel_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(UserChoiceRecord::from).collect())
    }
}

#[derive(Debug, FromRow)]
struct ChoiceWorldStateRow {
    user_id: Uuid,
    novel_id: Uuid,
    state: serde_json::Value,
    updated_at: DateTime<Utc>,
}

impl From<ChoiceWorldStateRow> for WorldState {
    fn from(row: ChoiceWorldStateRow) -> Self {
        Self {
            user_id: row.user_id,
            novel_id: row.novel_id,
            state: row.state,
            updated_at: row.updated_at,
        }
    }
}
