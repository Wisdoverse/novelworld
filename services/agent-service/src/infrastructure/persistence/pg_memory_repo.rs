use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::memory::{Memory, MemoryLayer};
use crate::domain::repositories::MemoryRepository;

#[derive(Debug, FromRow)]
struct MemoryRow {
    id: Uuid,
    character_id: Uuid,
    user_id: Uuid,
    novel_id: Uuid,
    layer: String,
    content: String,
    importance: i32,
    chapter_number: Option<i32>,
    // embedding is stored as bytea or vector; omitted for basic queries
    created_at: DateTime<Utc>,
}

impl From<MemoryRow> for Memory {
    fn from(r: MemoryRow) -> Self {
        let layer = match r.layer.as_str() {
            "short" => MemoryLayer::Short,
            "mid" => MemoryLayer::Mid,
            "long" => MemoryLayer::Long,
            "permanent" => MemoryLayer::Permanent,
            _ => MemoryLayer::Short,
        };
        Memory {
            id: r.id,
            character_id: r.character_id,
            user_id: r.user_id,
            novel_id: r.novel_id,
            layer,
            content: r.content,
            importance: r.importance,
            chapter_number: r.chapter_number,
            embedding: None,
            created_at: r.created_at,
        }
    }
}

fn layer_to_str(layer: &MemoryLayer) -> &'static str {
    match layer {
        MemoryLayer::Short => "short",
        MemoryLayer::Mid => "mid",
        MemoryLayer::Long => "long",
        MemoryLayer::Permanent => "permanent",
    }
}

pub struct PgMemoryRepository {
    pool: PgPool,
}

impl PgMemoryRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl MemoryRepository for PgMemoryRepository {
    async fn save(&self, memory: &Memory) -> Result<()> {
        // Format embedding as pgvector text literal when present
        let embedding_str: Option<String> = memory.embedding.as_ref().map(|emb| {
            format!(
                "[{}]",
                emb.iter()
                    .map(|f| f.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        });

        sqlx::query(
            r#"
            INSERT INTO character_memories (
                id, character_id, user_id, novel_id,
                layer, content, importance, chapter_number, embedding, created_at
            ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9::vector, $10)
            ON CONFLICT (id) DO UPDATE SET
                content = EXCLUDED.content,
                importance = EXCLUDED.importance,
                embedding = EXCLUDED.embedding
            "#,
        )
        .bind(memory.id)
        .bind(memory.character_id)
        .bind(memory.user_id)
        .bind(memory.novel_id)
        .bind(layer_to_str(&memory.layer))
        .bind(&memory.content)
        .bind(memory.importance)
        .bind(memory.chapter_number)
        .bind(embedding_str)
        .bind(memory.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_by_layer(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        layer: MemoryLayer,
    ) -> Result<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT id, character_id, user_id, novel_id,
                   layer, content, importance, chapter_number, created_at
            FROM character_memories
            WHERE character_id = $1 AND user_id = $2 AND novel_id = $3 AND layer = $4
            ORDER BY importance DESC, created_at DESC
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(layer_to_str(&layer))
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Memory::from).collect())
    }

    async fn search_similar(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        embedding: &[f32],
        limit: usize,
    ) -> Result<Vec<Memory>> {
        // Format the embedding vector as a pgvector-compatible string literal: [0.1,0.2,...]
        let embedding_str = format!(
            "[{}]",
            embedding
                .iter()
                .map(|f| f.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );

        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT id, character_id, user_id, novel_id,
                   layer::text AS layer, content, importance, chapter_number, created_at
            FROM character_memories
            WHERE character_id = $1
              AND user_id = $2
              AND embedding IS NOT NULL
            ORDER BY embedding <=> $3::vector
            LIMIT $4
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(&embedding_str)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Memory::from).collect())
    }
}
