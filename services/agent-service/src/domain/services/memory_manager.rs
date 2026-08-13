use anyhow::Result;
use std::sync::Arc;
use uuid::Uuid;

use crate::domain::entities::memory::{ChatMessage, Memory, MemoryLayer};
use crate::domain::ports::{EmbeddingGenerator, MessageCache, TextSummarizer};
use crate::domain::repositories::{ChatRepository, MemoryRepository};

const SHORT_TERM_LIMIT: usize = 10;
const MID_TERM_TRIGGER: usize = 20;
/// Maximum number of semantically similar memories to inject into context.
const SEMANTIC_SEARCH_LIMIT: usize = 5;
const MAX_MEMORY_BLOCK_CHARS: usize = 4_000;
const MAX_RECENT_MESSAGE_CHARS: usize = 1_000;
const MAX_SUMMARY_INPUT_CHARS: usize = 24_000;

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn build_summary_input(messages: &[ChatMessage]) -> String {
    let mut remaining = MAX_SUMMARY_INPUT_CHARS;
    let mut lines = Vec::new();
    for message in messages.iter().rev() {
        let prefix = format!("{}: ", message.role);
        let separator = usize::from(!lines.is_empty());
        let prefix_chars = prefix.chars().count() + separator;
        if prefix_chars >= remaining {
            break;
        }
        let content = truncate_chars(&message.content, remaining - prefix_chars);
        remaining -= prefix_chars + content.chars().count();
        lines.push(format!("{prefix}{content}"));
    }
    lines.reverse();
    lines.join("\n")
}

/// 4层记忆金字塔管理器（借鉴 project-lunar Crystal Memory）
pub struct MemoryManager {
    pub memory_repo: Arc<dyn MemoryRepository>,
    pub chat_repo: Arc<dyn ChatRepository>,
    pub cache: Arc<dyn MessageCache>,
    pub llm: Arc<dyn TextSummarizer>,
    pub embedding: Arc<dyn EmbeddingGenerator>,
}

impl MemoryManager {
    /// Build context with semantic search: embeds the user's current message,
    /// retrieves similar long-term memories, and injects them into the context.
    pub async fn build_context_with_semantic(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        current_chapter: i32,
        system_prompt: &str,
        user_message: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut messages: Vec<(String, String)> = vec![];

        // 1. 系统提示词（角色人格）
        messages.push(("system".into(), system_prompt.to_string()));

        // 2. 永久记忆注入（角色关系、重大选择）
        let permanent = self
            .memory_repo
            .find_by_layer(
                character_id,
                user_id,
                novel_id,
                MemoryLayer::Permanent,
                current_chapter,
                10,
                0,
            )
            .await?;
        if !permanent.is_empty() {
            let perm_context = permanent
                .iter()
                .take(10)
                .map(|m| format!("- {}", m.content))
                .collect::<Vec<_>>()
                .join("\n");
            messages.push((
                "system".into(),
                format!(
                    "## 你与读者的关系和重要记忆\n{}",
                    truncate_chars(&perm_context, MAX_MEMORY_BLOCK_CHARS)
                ),
            ));
        }

        // 3. 中期记忆（对话摘要）
        let mid = self
            .memory_repo
            .find_by_layer(
                character_id,
                user_id,
                novel_id,
                MemoryLayer::Mid,
                current_chapter,
                5,
                0,
            )
            .await?;
        if !mid.is_empty() {
            let mid_context = mid
                .iter()
                .take(5)
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            messages.push((
                "system".into(),
                format!(
                    "## 之前对话的摘要\n{}",
                    truncate_chars(&mid_context, MAX_MEMORY_BLOCK_CHARS)
                ),
            ));
        }

        // 3.5 Semantic search: embed the user message and retrieve similar long-term memories
        if let Ok(query_embedding) = self.embedding.generate_embedding(user_message).await {
            if let Ok(similar) = self
                .memory_repo
                .search_similar(
                    character_id,
                    user_id,
                    novel_id,
                    &query_embedding,
                    current_chapter,
                    SEMANTIC_SEARCH_LIMIT,
                )
                .await
            {
                if !similar.is_empty() {
                    let semantic_context = similar
                        .iter()
                        .map(|m| format!("- {}", m.content))
                        .collect::<Vec<_>>()
                        .join("\n");
                    messages.push((
                        "system".into(),
                        format!(
                            "## 相关记忆（语义检索）\n{}",
                            truncate_chars(&semantic_context, MAX_MEMORY_BLOCK_CHARS)
                        ),
                    ));
                }
            }
        }

        // 4. 防剧透：注入当前章节之前的故事背景
        messages.push(("system".into(), format!(
            "## 当前故事进度\n读者目前读到第{}章。你只知道第{}章及之前发生的事情，不要提及后续剧情。",
            current_chapter, current_chapter
        )));

        // 5. Recent committed turns. PostgreSQL is authoritative; the Redis
        // projection may lag or be empty after a restart.
        let recent = self
            .chat_repo
            .find_recent(
                character_id,
                user_id,
                novel_id,
                current_chapter,
                SHORT_TERM_LIMIT,
            )
            .await?;
        for msg in recent {
            let role = if msg.role == "user" {
                "user"
            } else {
                "assistant"
            };
            messages.push((
                role.into(),
                truncate_chars(&msg.content, MAX_RECENT_MESSAGE_CHARS),
            ));
        }

        Ok(messages)
    }

    /// Project an already committed turn into Redis and derived memories.
    pub async fn project_completed_turn(
        &self,
        user_msg: ChatMessage,
        char_msg: ChatMessage,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<()> {
        // PostgreSQL is the durable source of truth. This projection runs only
        // after the atomic turn transaction commits.
        let projected = self
            .cache
            .push_turn(character_id, user_id, &user_msg, &char_msg)
            .await?;
        if !projected {
            return Ok(());
        }

        // 检查是否需要触发中期记忆摘要
        let total_count = self
            .chat_repo
            .count(character_id, user_id, novel_id)
            .await?;

        if total_count % MID_TERM_TRIGGER == 0 {
            self.consolidate_to_mid_term(character_id, user_id, novel_id, char_msg.chapter_context)
                .await?;
        }

        Ok(())
    }

    /// 将最近 N 条对话摘要为中期记忆
    async fn consolidate_to_mid_term(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_context: Option<i32>,
    ) -> Result<()> {
        let recent = self
            .chat_repo
            .find_recent(
                character_id,
                user_id,
                novel_id,
                chapter_context.ok_or_else(|| anyhow::anyhow!("missing chapter context"))?,
                MID_TERM_TRIGGER,
            )
            .await?;

        let conversation = build_summary_input(&recent);

        let summary = self
            .llm
            .summarize(
                "你是一个对话摘要助手。请将以下对话压缩为2-3句话的摘要，保留关键信息和情感变化。",
                &conversation,
            )
            .await?;

        let memory = Memory {
            id: uuid::Uuid::new_v4(),
            character_id,
            user_id,
            novel_id,
            layer: MemoryLayer::Mid,
            content: summary,
            importance: 6,
            chapter_number: chapter_context,
            embedding: None,
            created_at: chrono::Utc::now(),
        };

        self.memory_repo.save(&memory).await?;
        Ok(())
    }

    /// 保存永久记忆（重大选择、关系变化）
    /// Generates an embedding for the event text so it can be found via semantic search.
    pub async fn save_permanent_memory(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        event: &str,
        importance: i32,
    ) -> Result<()> {
        // Attempt to generate an embedding; if it fails, save without one.
        let embedding = self.embedding.generate_embedding(event).await.ok();
        let mut memory = Memory::new_permanent(
            character_id,
            user_id,
            novel_id,
            event.to_string(),
            importance,
            chapter_number,
        );
        memory.embedding = embedding;
        self.memory_repo.save(&memory).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_input_keeps_recent_unicode_within_budget() {
        let user_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let messages = vec![
            ChatMessage::new(
                user_id,
                character_id,
                novel_id,
                "user".into(),
                "旧".repeat(MAX_SUMMARY_INPUT_CHARS),
                None,
                Some(1),
            ),
            ChatMessage::new(
                user_id,
                character_id,
                novel_id,
                "character".into(),
                "最新内容".into(),
                None,
                Some(1),
            ),
        ];

        let input = build_summary_input(&messages);
        assert!(input.chars().count() <= MAX_SUMMARY_INPUT_CHARS);
        assert!(input.contains("最新内容"));
    }
}
