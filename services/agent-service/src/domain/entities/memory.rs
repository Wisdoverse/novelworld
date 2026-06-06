use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 记忆层级（借鉴 project-lunar 的 Crystal Memory 4层金字塔）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MemoryLayer {
    /// 短期记忆：最近 N 条对话，存于 Redis，TTL=24h
    Short,
    /// 中期记忆：每 20 条对话自动摘要，存于 PostgreSQL
    Mid,
    /// 长期记忆：关键事件向量化，pgvector 语义检索
    Long,
    /// 永久记忆：角色关系状态、读者身份、重大选择，永不过期
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
    /// 重要程度 1-10，影响检索优先级
    pub importance: i32,
    pub chapter_number: Option<i32>,
    /// 长期记忆的向量嵌入（1536维，OpenAI text-embedding-3-small）
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
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            character_id,
            user_id,
            novel_id,
            layer: MemoryLayer::Permanent,
            content,
            importance,
            chapter_number: None,
            embedding: None,
            created_at: Utc::now(),
        }
    }
}

/// 对话消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: Uuid,
    pub user_id: Uuid,
    pub character_id: Uuid,
    pub novel_id: Uuid,
    /// "user" | "character"
    pub role: String,
    pub content: String,
    pub reader_identity: Option<String>,
    pub chapter_context: Option<i32>,
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
            user_id,
            character_id,
            novel_id,
            role,
            content,
            reader_identity,
            chapter_context,
            created_at: Utc::now(),
        }
    }
}
