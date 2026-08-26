use anyhow::{ensure, Result};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::prelude::FromRow;
use sqlx::PgPool;
use uuid::Uuid;

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
    importance: i16,
    chapter_number: Option<i32>,
    persona_source_chapter_high_water: Option<i32>,
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
            importance: i32::from(r.importance),
            chapter_number: r.chapter_number,
            persona_source_chapter_high_water: r.persona_source_chapter_high_water,
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

fn validate_persona_provenance(memory: &Memory, chapter_number: i32) -> Result<()> {
    match &memory.layer {
        MemoryLayer::Mid | MemoryLayer::Long => ensure!(
            memory
                .persona_source_chapter_high_water
                .is_some_and(|chapter| (1..=chapter_number).contains(&chapter)),
            "derived memory is missing safe persona provenance"
        ),
        MemoryLayer::Short | MemoryLayer::Permanent => ensure!(
            memory.persona_source_chapter_high_water.is_none(),
            "non-derived memory cannot carry persona provenance"
        ),
    }
    Ok(())
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
    async fn insert_if_absent(&self, memory: &Memory) -> Result<bool> {
        let chapter_number = memory
            .chapter_number
            .ok_or_else(|| anyhow::anyhow!("memory chapter_number is required"))?;
        validate_persona_provenance(memory, chapter_number)?;
        let result = sqlx::query(
            r#"
            INSERT INTO character_memories (
                id, character_id, user_id, novel_id,
                layer, content, importance, chapter_number,
                persona_source_chapter_high_water, embedding, created_at
            ) VALUES ($1, $2, $3, $4, $5::memory_layer, $6, $7, $8, $9, NULL, $10)
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(memory.id)
        .bind(memory.character_id)
        .bind(memory.user_id)
        .bind(memory.novel_id)
        .bind(layer_to_str(&memory.layer))
        .bind(&memory.content)
        .bind(i16::try_from(memory.importance)?)
        .bind(chapter_number)
        .bind(memory.persona_source_chapter_high_water)
        .bind(memory.created_at)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() == 1)
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<Memory>> {
        let row = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT id, character_id, user_id, novel_id,
                   layer::text AS layer, content, importance, chapter_number,
                   persona_source_chapter_high_water, created_at
            FROM character_memories
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(Memory::from))
    }

    async fn save(&self, memory: &Memory) -> Result<()> {
        let chapter_number = memory
            .chapter_number
            .ok_or_else(|| anyhow::anyhow!("memory chapter_number is required"))?;
        validate_persona_provenance(memory, chapter_number)?;
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
                layer, content, importance, chapter_number,
                persona_source_chapter_high_water, embedding, created_at
            ) VALUES ($1, $2, $3, $4, $5::memory_layer, $6, $7, $8, $9, $10::vector, $11)
            ON CONFLICT (id) DO UPDATE SET
                content = EXCLUDED.content,
                importance = EXCLUDED.importance,
                persona_source_chapter_high_water = EXCLUDED.persona_source_chapter_high_water,
                embedding = EXCLUDED.embedding
            "#,
        )
        .bind(memory.id)
        .bind(memory.character_id)
        .bind(memory.user_id)
        .bind(memory.novel_id)
        .bind(layer_to_str(&memory.layer))
        .bind(&memory.content)
        .bind(i16::try_from(memory.importance)?)
        .bind(chapter_number)
        .bind(memory.persona_source_chapter_high_water)
        .bind(embedding_str)
        .bind(memory.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn find_by_layer(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        layer: MemoryLayer,
        max_chapter: i32,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            SELECT id, character_id, user_id, novel_id,
                   layer::text AS layer, content, importance, chapter_number,
                   persona_source_chapter_high_water, created_at
            FROM character_memories
            WHERE character_id = $1 AND user_id = $2 AND novel_id = $3
              AND layer = $4::memory_layer
              AND chapter_number IS NOT NULL AND chapter_number <= $5
              AND (
                    layer NOT IN ('mid'::memory_layer, 'long'::memory_layer)
                    OR persona_source_chapter_high_water BETWEEN 1 AND $5
              )
            ORDER BY importance DESC, created_at DESC
            LIMIT $6 OFFSET $7
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(layer_to_str(&layer))
        .bind(max_chapter)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Memory::from).collect())
    }

    async fn find_permanent_candidates(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        max_chapter: i32,
        journey_limit: i64,
        legacy_limit: i64,
    ) -> Result<Vec<Memory>> {
        let rows = sqlx::query_as::<_, MemoryRow>(
            r#"
            WITH journey_candidates AS (
                SELECT id, character_id, user_id, novel_id,
                       layer::text AS layer, content, importance, chapter_number,
                       persona_source_chapter_high_water, created_at,
                       0 AS candidate_bucket
                FROM character_memories
                WHERE character_id = $1 AND user_id = $2 AND novel_id = $3
                  AND layer = 'permanent'::memory_layer
                  AND chapter_number IS NOT NULL AND chapter_number <= $4
                  AND (get_byte(uuid_send(id), 6) >> 4) = 5
                ORDER BY created_at DESC, id DESC
                LIMIT $5
            ),
            legacy_candidates AS (
                SELECT id, character_id, user_id, novel_id,
                       layer::text AS layer, content, importance, chapter_number,
                       persona_source_chapter_high_water, created_at,
                       1 AS candidate_bucket
                FROM character_memories
                WHERE character_id = $1 AND user_id = $2 AND novel_id = $3
                  AND layer = 'permanent'::memory_layer
                  AND chapter_number IS NOT NULL AND chapter_number <= $4
                  AND (get_byte(uuid_send(id), 6) >> 4) <> 5
                  AND NOT (
                      (get_byte(uuid_send(id), 6) >> 4) = 4
                      AND importance = 7
                  )
                ORDER BY importance DESC, created_at DESC, id DESC
                LIMIT $6
            )
            SELECT id, character_id, user_id, novel_id,
                   layer, content, importance, chapter_number,
                   persona_source_chapter_high_water, created_at
            FROM (
                SELECT * FROM journey_candidates
                UNION ALL
                SELECT * FROM legacy_candidates
            ) AS candidates
            ORDER BY candidate_bucket, importance DESC, created_at DESC, id DESC
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(max_chapter)
        .bind(journey_limit)
        .bind(legacy_limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Memory::from).collect())
    }

    async fn search_similar(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        embedding: &[f32],
        max_chapter: i32,
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
                   layer::text AS layer, content, importance, chapter_number,
                   persona_source_chapter_high_water, created_at
            FROM character_memories
            WHERE character_id = $1
              AND user_id = $2
              AND novel_id = $3
              AND chapter_number IS NOT NULL
              AND chapter_number <= $5
              AND embedding IS NOT NULL
              AND layer IN ('long', 'permanent')
              AND (
                    layer = 'permanent'::memory_layer
                    OR persona_source_chapter_high_water BETWEEN 1 AND $5
              )
              AND NOT (
                  layer = 'permanent'::memory_layer
                  AND importance = 7
                  AND (get_byte(uuid_send(id), 6) >> 4) = 4
              )
            ORDER BY embedding <=> $4::vector
            LIMIT $6
            "#,
        )
        .bind(character_id)
        .bind(user_id)
        .bind(novel_id)
        .bind(&embedding_str)
        .bind(max_chapter)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(Memory::from).collect())
    }
}
