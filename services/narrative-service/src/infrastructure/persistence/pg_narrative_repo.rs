use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::narrative_node::{NarrativeNode, NarrativeChoice};
use crate::domain::repositories::{NarrativeNodeRepository, UserChoiceRepository, UserChoiceRecord};

// ─── NarrativeNode persistence ──────────────────────────────────────────────

#[derive(Debug, FromRow)]
struct NarrativeNodeRow {
    id: Uuid,
    novel_id: Uuid,
    chapter_id: Uuid,
    chapter_number: i32,
    description: String,
    choices: serde_json::Value,
    created_at: DateTime<Utc>,
}

impl From<NarrativeNodeRow> for NarrativeNode {
    fn from(r: NarrativeNodeRow) -> Self {
        let choices: Vec<NarrativeChoice> =
            serde_json::from_value(r.choices).unwrap_or_default();
        NarrativeNode {
            id: r.id,
            novel_id: r.novel_id,
            chapter_id: r.chapter_id,
            chapter_number: r.chapter_number,
            description: r.description,
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
                id, novel_id, chapter_id, chapter_number,
                description, choices, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (novel_id, chapter_number) DO UPDATE SET
                description = EXCLUDED.description,
                choices = EXCLUDED.choices
            "#,
        )
        .bind(node.id)
        .bind(node.novel_id)
        .bind(node.chapter_id)
        .bind(node.chapter_number)
        .bind(&node.description)
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
        let row = sqlx::query_as::<_, NarrativeNodeRow>(
            "SELECT * FROM narrative_nodes WHERE id = $1",
        )
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
    async fn save_choice(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
        node_id: Uuid,
        chapter_number: i32,
        choice_index: i32,
        choice_text: &str,
        consequence: Option<&str>,
    ) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO user_choices (
                id, user_id, novel_id, node_id, chapter_number,
                choice_index, choice_text, consequence, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            "#,
        )
        .bind(Uuid::new_v4())
        .bind(user_id)
        .bind(novel_id)
        .bind(node_id)
        .bind(chapter_number)
        .bind(choice_index)
        .bind(choice_text)
        .bind(consequence)
        .bind(Utc::now())
        .execute(&self.pool)
        .await?;
        Ok(())
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

    async fn find_by_novel(
        &self,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<Vec<UserChoiceRecord>> {
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
