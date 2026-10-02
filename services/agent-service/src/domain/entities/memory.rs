use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Memory layers, based on project-lunar's four-layer Crystal Memory pyramid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MemoryLayer {
    /// Short-term memory: the latest N committed conversations in PostgreSQL; Redis is only an optional projection.
    Short,
    /// Mid-term memory: automatically summarized every 20 conversations and stored in PostgreSQL.
    Mid,
    /// Long-term memory: conversation summaries are embedded, stored in PostgreSQL, and queried with pgvector.
    Long,
    /// Permanent memory: character relationships, reader identity, and major choices; never expires.
    Permanent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: Uuid,
    pub character_id: Uuid,
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub layer: MemoryLayer,
    pub content: String,
    /// Importance from 1 to 10; affects retrieval priority.
    pub importance: i32,
    pub chapter_number: Option<i32>,
    /// Highest persona source chapter represented by a derived Mid/Long row.
    /// Legacy rows are unmarked and must stay out of online prompt paths.
    #[serde(default, skip_serializing)]
    pub persona_source_chapter_high_water: Option<i32>,
    /// Long-term memory embedding (fixed at 1,536 dimensions).
    pub embedding: Option<Vec<f32>>,
    pub created_at: DateTime<Utc>,
}

impl Memory {
    pub fn new_short(
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        content: String,
        chapter_number: Option<i32>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            character_id,
            user_id,
            novel_id,
            layer: MemoryLayer::Short,
            content,
            importance: 5,
            chapter_number,
            persona_source_chapter_high_water: None,
            embedding: None,
            created_at: Utc::now(),
        }
    }

    pub fn new_permanent(
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        content: String,
        importance: i32,
        chapter_number: i32,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            character_id,
            user_id,
            novel_id,
            layer: MemoryLayer::Permanent,
            content,
            importance,
            chapter_number: Some(chapter_number),
            persona_source_chapter_high_water: None,
            embedding: None,
            created_at: Utc::now(),
        }
    }
}

/// Chat message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: Uuid,
    pub turn_id: Option<Uuid>,
    pub user_id: Uuid,
    pub character_id: Uuid,
    pub novel_id: Uuid,
    /// "user" | "character"
    pub role: String,
    pub content: String,
    pub reader_identity: Option<String>,
    pub chapter_context: Option<i32>,
    /// Internal provenance projected from the owning durable chat turn.
    #[serde(default, skip_serializing)]
    pub persona_source_chapter_high_water: Option<i32>,
    pub created_at: DateTime<Utc>,
}

impl ChatMessage {
    pub fn new(
        user_id: Uuid,
        character_id: Uuid,
        novel_id: Uuid,
        role: String,
        content: String,
        reader_identity: Option<String>,
        chapter_context: Option<i32>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            turn_id: None,
            user_id,
            character_id,
            novel_id,
            role,
            content,
            reader_identity,
            chapter_context,
            persona_source_chapter_high_water: None,
            created_at: Utc::now(),
        }
    }

    pub fn with_turn_id(mut self, turn_id: Uuid) -> Self {
        self.turn_id = Some(turn_id);
        self
    }
}
