use async_trait::async_trait;
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::repositories::{CharacterInfo, CharacterInfoRepository};

/// Row returned from the characters table (shared with novel-service).
/// We query only the fields needed to build the system prompt for agent chat.
#[derive(Debug, FromRow)]
struct CharacterInfoRow {
    id: Uuid,
    novel_id: Uuid,
    name: String,
    personality: Option<String>,
    background: Option<String>,
    speaking_style: Option<String>,
    description: Option<String>,
    system_prompt: Option<String>,
    // Novel fields (from JOIN)
    novel_title: Option<String>,
    world_summary: Option<String>,
}

impl CharacterInfoRow {
    /// Build a system prompt from character fields if no pre-built prompt exists.
    /// Mirrors Character::build_system_prompt() from novel-service.
    fn build_system_prompt(&self) -> String {
        let novel_title = self.novel_title.as_deref().unwrap_or("Unknown");
        let world_summary = self.world_summary.as_deref().unwrap_or("");
        let personality = self.personality.as_deref().unwrap_or("Unknown");
        let background = self.background.as_deref().unwrap_or("Unknown");
        let speaking_style = self.speaking_style.as_deref().unwrap_or("Natural");
        let description = self.description.as_deref().unwrap_or("");

        format!(
            r#"You are the character "{name}" from "{novel_title}".

## World Background
{world_summary}

## Your Character Info
- **Name**: {name}
- **Description**: {description}
- **Personality**: {personality}
- **Background**: {background}
- **Speaking Style**: {speaking_style}

## Behavioral Rules
1. Always respond in character as "{name}", maintaining consistency
2. Your speech patterns and word choices should match the character
3. You only know events you have experienced in the story (anti-spoiler rule)
4. Remember your conversation history with the reader for relationship continuity
5. Keep responses natural and immersive; do not break the fourth wall
6. If asked about information you should not know, deflect naturally in character"#,
            novel_title = novel_title,
            name = self.name,
            world_summary = world_summary,
            description = description,
            personality = personality,
            background = background,
            speaking_style = speaking_style,
        )
    }
}

pub struct PgCharacterInfoRepository {
    pool: PgPool,
}

impl PgCharacterInfoRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl CharacterInfoRepository for PgCharacterInfoRepository {
    async fn find_by_id(&self, id: Uuid) -> Result<Option<CharacterInfo>> {
        let row = sqlx::query_as::<_, CharacterInfoRow>(
            r#"
            SELECT
                c.id, c.novel_id, c.name,
                c.personality, c.background, c.speaking_style,
                c.description, c.system_prompt,
                n.title AS novel_title,
                n.world_summary
            FROM characters c
            LEFT JOIN novels n ON n.id = c.novel_id
            WHERE c.id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            // Use pre-built system_prompt if available, otherwise build from fields
            let system_prompt = r.system_prompt.clone()
                .or_else(|| Some(r.build_system_prompt()));
            CharacterInfo {
                id: r.id,
                name: r.name.clone(),
                novel_id: r.novel_id,
                system_prompt,
            }
        }))
    }
}
