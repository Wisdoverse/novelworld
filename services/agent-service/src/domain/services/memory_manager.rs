use std::sync::Arc;
use anyhow::Result;
use uuid::Uuid;

use crate::domain::entities::memory::{Memory, MemoryLayer, ChatMessage};
use crate::domain::repositories::{MemoryRepository, ChatRepository};
use crate::domain::ports::{MessageCache, TextSummarizer, EmbeddingGenerator};

const SHORT_TERM_LIMIT: usize = 10;
const MID_TERM_TRIGGER: usize = 20;
/// Maximum number of semantically similar memories to inject into context.
const SEMANTIC_SEARCH_LIMIT: usize = 5;

/// 4层记忆金字塔管理器（借鉴 project-lunar Crystal Memory）
pub struct MemoryManager {
    pub memory_repo: Arc<dyn MemoryRepository>,
    pub chat_repo: Arc<dyn ChatRepository>,
    pub cache: Arc<dyn MessageCache>,
    pub llm: Arc<dyn TextSummarizer>,
    pub embedding: Arc<dyn EmbeddingGenerator>,
}

impl MemoryManager {
    /// 构建 Agent 对话时的完整上下文
    /// 防剧透：只注入 current_chapter 之前的记忆
    pub async fn build_context(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        current_chapter: i32,
        system_prompt: &str,
    ) -> Result<Vec<(String, String)>> {
        let mut messages: Vec<(String, String)> = vec![];

        // 1. 系统提示词（角色人格）
        messages.push(("system".into(), system_prompt.to_string()));

        // 2. 永久记忆注入（角色关系、重大选择）
        let permanent = self.memory_repo
            .find_by_layer(character_id, user_id, novel_id, MemoryLayer::Permanent)
            .await?;
        if !permanent.is_empty() {
            let perm_context = permanent.iter()
                .map(|m| format!("- {}", m.content))
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(("system".into(), format!(
                "## 你与读者的关系和重要记忆\n{}", perm_context
            )));
        }

        // 3. 中期记忆（对话摘要）
        let mid = self.memory_repo
            .find_by_layer(character_id, user_id, novel_id, MemoryLayer::Mid)
            .await?;
        if !mid.is_empty() {
            let mid_context = mid.iter()
                .take(5) // 最多5条摘要
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(("system".into(), format!(
                "## 之前对话的摘要\n{}", mid_context
            )));
        }

        // 4. 防剧透：注入当前章节之前的故事背景
        messages.push(("system".into(), format!(
            "## 当前故事进度\n读者目前读到第{}章。你只知道第{}章及之前发生的事情，不要提及后续剧情。",
            current_chapter, current_chapter
        )));

        // 5. 短期记忆（最近对话，从缓存获取）
        let recent = self.cache
            .get_recent_messages(character_id, user_id, SHORT_TERM_LIMIT)
            .await?;
        for msg in recent {
            let role = if msg.role == "user" { "user" } else { "assistant" };
            messages.push((role.into(), msg.content));
        }

        Ok(messages)
    }

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
        let permanent = self.memory_repo
            .find_by_layer(character_id, user_id, novel_id, MemoryLayer::Permanent)
            .await?;
        if !permanent.is_empty() {
            let perm_context = permanent.iter()
                .map(|m| format!("- {}", m.content))
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(("system".into(), format!(
                "## 你与读者的关系和重要记忆\n{}", perm_context
            )));
        }

        // 3. 中期记忆（对话摘要）
        let mid = self.memory_repo
            .find_by_layer(character_id, user_id, novel_id, MemoryLayer::Mid)
            .await?;
        if !mid.is_empty() {
            let mid_context = mid.iter()
                .take(5)
                .map(|m| m.content.as_str())
                .collect::<Vec<_>>()
                .join("\n");
            messages.push(("system".into(), format!(
                "## 之前对话的摘要\n{}", mid_context
            )));
        }

        // 3.5 Semantic search: embed the user message and retrieve similar long-term memories
        if let Ok(query_embedding) = self.embedding.generate_embedding(user_message).await {
            if let Ok(similar) = self.memory_repo
                .search_similar(character_id, user_id, &query_embedding, SEMANTIC_SEARCH_LIMIT)
                .await
            {
                if !similar.is_empty() {
                    let semantic_context = similar.iter()
                        .map(|m| format!("- {}", m.content))
                        .collect::<Vec<_>>()
                        .join("\n");
                    messages.push(("system".into(), format!(
                        "## 相关记忆（语义检索）\n{}", semantic_context
                    )));
                }
            }
        }

        // 4. 防剧透：注入当前章节之前的故事背景
        messages.push(("system".into(), format!(
            "## 当前故事进度\n读者目前读到第{}章。你只知道第{}章及之前发生的事情，不要提及后续剧情。",
            current_chapter, current_chapter
        )));

        // 5. 短期记忆（最近对话，从缓存获取）
        let recent = self.cache
            .get_recent_messages(character_id, user_id, SHORT_TERM_LIMIT)
            .await?;
        for msg in recent {
            let role = if msg.role == "user" { "user" } else { "assistant" };
            messages.push((role.into(), msg.content));
        }

        Ok(messages)
    }

    /// 保存新消息并触发记忆整合
    pub async fn save_and_consolidate(
        &self,
        user_msg: ChatMessage,
        char_msg: ChatMessage,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<()> {
        // 保存到数据库
        self.chat_repo.save(&user_msg).await?;
        self.chat_repo.save(&char_msg).await?;

        // 保存到短期缓存
        self.cache.push_message(character_id, user_id, &user_msg).await?;
        self.cache.push_message(character_id, user_id, &char_msg).await?;

        // 检查是否需要触发中期记忆摘要
        let total_count = self.chat_repo
            .count(character_id, user_id, novel_id)
            .await?;

        if total_count % MID_TERM_TRIGGER == 0 {
            self.consolidate_to_mid_term(character_id, user_id, novel_id).await?;
        }

        Ok(())
    }

    /// 将最近 N 条对话摘要为中期记忆
    async fn consolidate_to_mid_term(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
    ) -> Result<()> {
        let recent = self.chat_repo
            .find_recent(character_id, user_id, novel_id, MID_TERM_TRIGGER)
            .await?;

        let conversation = recent.iter()
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");

        let summary = self.llm.summarize(
            "你是一个对话摘要助手。请将以下对话压缩为2-3句话的摘要，保留关键信息和情感变化。",
            &conversation,
        ).await?;

        let memory = Memory {
            id: uuid::Uuid::new_v4(),
            character_id,
            user_id,
            novel_id,
            layer: MemoryLayer::Mid,
            content: summary,
            importance: 6,
            chapter_number: None,
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
        event: &str,
        importance: i32,
    ) -> Result<()> {
        // Attempt to generate an embedding; if it fails, save without one.
        let embedding = self.embedding.generate_embedding(event).await.ok();
        let mut memory = Memory::new_permanent(
            character_id, user_id, novel_id,
            event.to_string(), importance,
        );
        memory.embedding = embedding;
        self.memory_repo.save(&memory).await?;
        Ok(())
    }
}
