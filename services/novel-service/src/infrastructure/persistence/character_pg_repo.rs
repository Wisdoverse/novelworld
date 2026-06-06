use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::character::Character;
use crate::domain::repositories::{CharacterRepository, CharacterRelationshipRecord};
use crate::domain::value_objects::{CharacterRole, AvatarStatus};

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

#[async_trait]
impl CharacterRepository for CharacterPgRepository {
    async fn save_batch(&self, characters: &[Character]) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        for ch in characters {
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
                ON CONFLICT (id) DO UPDATE SET
                    name = EXCLUDED.name,
                    aliases = EXCLUDED.aliases,
                    role = EXCLUDED.role,
                    description = EXCLUDED.description,
                    personality = EXCLUDED.personality,
                    background = EXCLUDED.background,
                    speaking_style = EXCLUDED.speaking_style,
                    appearance = EXCLUDED.appearance,
                    avatar_url = EXCLUDED.avatar_url,
                    avatar_status = EXCLUDED.avatar_status,
                    first_appearance_chapter = EXCLUDED.first_appearance_chapter,
                    updated_at = EXCLUDED.updated_at
                "#,
            )
            .bind(ch.id)
            .bind(ch.novel_id)
            .bind(&ch.name)
            .bind(&ch.aliases)
            .bind(ch.role.to_str())
            .bind(&ch.description)
            .bind(&ch.personality)
            .bind(&ch.background)
            .bind(&ch.speaking_style)
            .bind(&ch.appearance)
            .bind(&ch.avatar_url)
            .bind(ch.avatar_status.to_str())
            .bind(ch.first_appearance_chapter)
            .bind(ch.created_at)
            .bind(ch.updated_at)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
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

    async fn update(&self, character: &Character) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE characters SET
                name = $2, aliases = $3,
                role = $4::text::character_role,
                description = $5, personality = $6,
                background = $7, speaking_style = $8,
                appearance = $9, avatar_url = $10,
                avatar_status = $11::text::avatar_status,
                first_appearance_chapter = $12,
                updated_at = $13
            WHERE id = $1
            "#,
        )
        .bind(character.id)
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
        .bind(character.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn save_relationship(
        &self,
        novel_id: Uuid,
        from_id: Uuid,
        to_id: Uuid,
        rel_type: &str,
        description: Option<&str>,
        strength: i32,
    ) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO character_relationships (id, novel_id, from_character_id, to_character_id, relationship_type, description, strength)
               VALUES ($1, $2, $3, $4, $5, $6, $7)"#
        )
        .bind(Uuid::new_v4())
        .bind(novel_id)
        .bind(from_id)
        .bind(to_id)
        .bind(rel_type)
        .bind(description)
        .bind(strength as i16)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_relationships(&self, novel_id: Uuid) -> Result<Vec<CharacterRelationshipRecord>> {
        let rows = sqlx::query_as::<_, RelRow>(
            "SELECT id, novel_id, from_character_id, to_character_id, relationship_type, description, strength FROM character_relationships WHERE novel_id = $1"
        )
        .bind(novel_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows.into_iter().map(|r| CharacterRelationshipRecord {
            id: r.id,
            novel_id: r.novel_id,
            from_character_id: r.from_character_id,
            to_character_id: r.to_character_id,
            relationship_type: r.relationship_type,
            description: r.description,
            strength: r.strength as i32,
        }).collect())
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
