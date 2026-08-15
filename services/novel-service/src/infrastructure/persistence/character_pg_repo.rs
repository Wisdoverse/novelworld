use std::collections::HashSet;

use anyhow::{ensure, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::entities::character::Character;
use crate::domain::repositories::{CharacterRelationshipRecord, CharacterRepository};
use crate::domain::value_objects::{AvatarStatus, CharacterRole};

#[derive(Debug, FromRow)]
struct CharacterRow {
    id: Uuid,
    novel_id: Uuid,
    name: String,
    aliases: Vec<String>,
    role: String,
    description: Option<String>,
    personality: Option<String>,
    background: Option<String>,
    speaking_style: Option<String>,
    appearance: Option<String>,
    avatar_url: Option<String>,
    avatar_status: String,
    first_appearance_chapter: Option<i32>,
    #[allow(dead_code)]
    traits: serde_json::Value,
    created_at: Option<DateTime<Utc>>,
    updated_at: Option<DateTime<Utc>>,
}

impl From<CharacterRow> for Character {
    fn from(r: CharacterRow) -> Self {
        let now = Utc::now();
        Character {
            id: r.id,
            novel_id: r.novel_id,
            name: r.name,
            aliases: r.aliases,
            role: CharacterRole::from_str(&r.role),
            description: r.description,
            personality: r.personality,
            background: r.background,
            speaking_style: r.speaking_style,
            appearance: r.appearance,
            avatar_url: r.avatar_url,
            avatar_status: AvatarStatus::from_str(&r.avatar_status),
            system_prompt: None,
            first_appearance_chapter: r.first_appearance_chapter,
            created_at: r.created_at.unwrap_or(now),
            updated_at: r.updated_at.unwrap_or(now),
        }
    }
}

pub struct CharacterPgRepository {
    pool: PgPool,
}

impl CharacterPgRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

async fn save_batch_in_transaction(
    transaction: &mut Transaction<'_, Postgres>,
    characters: &[Character],
) -> Result<()> {
    for character in characters {
        sqlx::query(
            r#"
            INSERT INTO characters (
                id, novel_id, name, aliases, role,
                description, personality, background,
                speaking_style, appearance,
                avatar_url, avatar_status,
                first_appearance_chapter,
                created_at, updated_at
            ) VALUES (
                $1, $2, $3, $4, $5::text::character_role,
                $6, $7, $8, $9, $10,
                $11, $12::text::avatar_status,
                $13, $14, $15
            )
            "#,
        )
        .bind(character.id)
        .bind(character.novel_id)
        .bind(&character.name)
        .bind(&character.aliases)
        .bind(character.role.to_str())
        .bind(&character.description)
        .bind(&character.personality)
        .bind(&character.background)
        .bind(&character.speaking_style)
        .bind(&character.appearance)
        .bind(&character.avatar_url)
        .bind(character.avatar_status.to_str())
        .bind(character.first_appearance_chapter)
        .bind(character.created_at)
        .bind(character.updated_at)
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

#[async_trait]
impl CharacterRepository for CharacterPgRepository {
    async fn replace_import(
        &self,
        novel_id: Uuid,
        attempt: i64,
        characters: &[Character],
        relationships: &[CharacterRelationshipRecord],
    ) -> Result<bool> {
        ensure!(!characters.is_empty(), "import requires characters");
        let character_ids: HashSet<_> = characters.iter().map(|character| character.id).collect();
        ensure!(
            character_ids.len() == characters.len()
                && characters
                    .iter()
                    .all(|character| character.novel_id == novel_id),
            "import characters are invalid"
        );
        ensure!(
            relationships.iter().all(|relationship| {
                relationship.novel_id == novel_id
                    && character_ids.contains(&relationship.from_character_id)
                    && character_ids.contains(&relationship.to_character_id)
                    && (0..=100).contains(&relationship.strength)
                    && !relationship.relationship_type.trim().is_empty()
                    && relationship.relationship_type.chars().count() <= 50
                    && !relationship.relationship_type.chars().any(char::is_control)
            }),
            "import relationships are invalid"
        );

        let mut transaction = self.pool.begin().await?;
        let fenced = sqlx::query_scalar::<_, bool>(
            "SELECT TRUE FROM novel_import_jobs \
             WHERE novel_id = $1 AND attempt = $2 AND status = 'in_progress' \
               AND stage = 'chapters' \
             FOR UPDATE",
        )
        .bind(novel_id)
        .bind(attempt)
        .fetch_optional(&mut *transaction)
        .await?
        .unwrap_or(false);
        if !fenced {
            return Ok(false);
        }

        sqlx::query("DELETE FROM characters WHERE novel_id = $1")
            .bind(novel_id)
            .execute(&mut *transaction)
            .await?;
        save_batch_in_transaction(&mut transaction, characters).await?;
        for relationship in relationships {
            sqlx::query(
                "INSERT INTO character_relationships ( \
                    id, novel_id, from_character_id, to_character_id, \
                    relationship_type, description, strength \
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(relationship.id)
            .bind(relationship.novel_id)
            .bind(relationship.from_character_id)
            .bind(relationship.to_character_id)
            .bind(&relationship.relationship_type)
            .bind(&relationship.description)
            .bind(relationship.strength as i16)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(true)
    }

    async fn find_by_novel(&self, novel_id: Uuid) -> Result<Vec<Character>> {
        let rows = sqlx::query_as::<_, CharacterRow>(
            r#"
            SELECT
                id, novel_id, name, aliases,
                role::text AS role,
                description, personality, background,
                speaking_style, appearance,
                avatar_url,
                avatar_status::text AS avatar_status,
                first_appearance_chapter,
                traits,
                created_at, updated_at
            FROM characters
            WHERE novel_id = $1
            ORDER BY first_appearance_chapter ASC NULLS LAST, name ASC
            "#,
        )
        .bind(novel_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Character::from).collect())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Character>> {
        let row = sqlx::query_as::<_, CharacterRow>(
            r#"
            SELECT
                id, novel_id, name, aliases,
                role::text AS role,
                description, personality, background,
                speaking_style, appearance,
                avatar_url,
                avatar_status::text AS avatar_status,
                first_appearance_chapter,
                traits,
                created_at, updated_at
            FROM characters
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(Character::from))
    }

    async fn set_avatar(&self, character_id: Uuid, avatar_url: &str) -> Result<()> {
        let result = sqlx::query(
            "UPDATE characters \
             SET avatar_url = $2, avatar_status = 'ready'::avatar_status, updated_at = NOW() \
             WHERE id = $1",
        )
        .bind(character_id)
        .bind(avatar_url)
        .execute(&self.pool)
        .await?;
        ensure!(result.rows_affected() == 1, "character does not exist");
        Ok(())
    }

    async fn find_relationships(&self, novel_id: Uuid) -> Result<Vec<CharacterRelationshipRecord>> {
        let rows = sqlx::query_as::<_, RelRow>(
            "SELECT id, novel_id, from_character_id, to_character_id, relationship_type, description, strength FROM character_relationships WHERE novel_id = $1"
        )
        .bind(novel_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| CharacterRelationshipRecord {
                id: r.id,
                novel_id: r.novel_id,
                from_character_id: r.from_character_id,
                to_character_id: r.to_character_id,
                relationship_type: r.relationship_type,
                description: r.description,
                strength: r.strength as i32,
            })
            .collect())
    }
}

#[derive(sqlx::FromRow)]
struct RelRow {
    id: Uuid,
    novel_id: Uuid,
    from_character_id: Uuid,
    to_character_id: Uuid,
    relationship_type: String,
    description: Option<String>,
    strength: i16,
}
