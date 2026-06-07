use std::sync::Arc;
use anyhow::Result;
use uuid::Uuid;
use futures::{Stream, StreamExt};

use crate::domain::entities::memory::{ChatMessage, Memory};
use crate::domain::services::memory_manager::MemoryManager;
use crate::domain::repositories::CharacterInfoRepository;
use crate::infrastructure::llm::LlmAdapter;

pub struct AgentCommandHandler {
    pub memory_manager: Arc<MemoryManager>,
    pub character_repo: Arc<dyn CharacterInfoRepository>,
    pub llm: Arc<LlmAdapter>,
}

impl AgentCommandHandler {
    /// 流式对话（SSE）
    ///
    /// Fix: replaced sleep-based save with oneshot channel that fires after stream
    /// completes, ensuring the full response is captured before persisting.
    pub async fn chat_stream(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        user_message: String,
        reader_identity: Option<String>,
        current_chapter: i32,
    ) -> Result<impl Stream<Item = Result<String>>> {
        // 获取角色信息（system prompt）
        let character = self.character_repo
            .find_by_id(character_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Character not found: {}", character_id))?;

        let system_prompt = character.system_prompt
            .unwrap_or_else(|| format!("你是角色 {}。", character.name));

        // 构建完整上下文（4层记忆 + 语义搜索）
        let mut context = self.memory_manager.build_context_with_semantic(
            character_id, user_id, novel_id,
            current_chapter, &system_prompt, &user_message,
        ).await?;

        // 注入读者身份
        if let Some(ref identity) = reader_identity {
            context.push(("system".into(), format!(
                "## 读者身份\n与你对话的读者身份是：{}。请根据这个身份调整你的称呼和互动方式。",
                identity
            )));
        }

        // 添加当前用户消息
        context.push(("user".into(), user_message.clone()));

        // 调用 LLM 流式接口
        let stream = self.llm.chat_stream(context).await?;

        // Prepare the user message for saving after stream completes
        let mm = self.memory_manager.clone();
        let user_msg = ChatMessage::new(
            user_id, character_id, novel_id,
            "user".into(), user_message,
            reader_identity.clone(), Some(current_chapter),
        );

        // Use a oneshot channel to signal stream completion with the full response.
        // The sender fires when the stream ends (either naturally or on error),
        // ensuring save_and_consolidate runs only after all chunks are collected.
        let (tx, rx) = tokio::sync::oneshot::channel::<String>();
        let full_response = Arc::new(tokio::sync::Mutex::new(String::new()));
        let full_response_writer = full_response.clone();

        let wrapped_stream = async_stream::stream! {
            let mut inner = Box::pin(stream);
            while let Some(chunk) = inner.next().await {
                if let Ok(ref text) = chunk {
                    full_response_writer.lock().await.push_str(text);
                }
                yield chunk;
            }
            // Stream exhausted: send the complete response to the save task
            let final_text = full_response_writer.lock().await.clone();
            let _ = tx.send(final_text);
        };

        // Spawn a task that waits for the stream to finish, then saves
        let reader_identity_clone = reader_identity.clone();
        tokio::spawn(async move {
            if let Ok(response_text) = rx.await {
                if !response_text.is_empty() {
                    let char_msg = ChatMessage::new(
                        user_id, character_id, novel_id,
                        "character".into(), response_text,
                        reader_identity_clone, Some(current_chapter),
                    );
                    if let Err(e) = mm.save_and_consolidate(
                        user_msg, char_msg,
                        character_id, user_id, novel_id,
                    ).await {
                        tracing::error!("Failed to save chat after stream: {}", e);
                    }
                }
            }
        });

        Ok(wrapped_stream)
    }

    /// 普通对话（非流式）
    pub async fn chat(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        user_message: String,
        reader_identity: Option<String>,
        current_chapter: i32,
    ) -> Result<String> {
        let character = self.character_repo
            .find_by_id(character_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Character not found"))?;

        let system_prompt = character.system_prompt
            .unwrap_or_else(|| format!("你是角色 {}。", character.name));

        let mut context = self.memory_manager.build_context_with_semantic(
            character_id, user_id, novel_id,
            current_chapter, &system_prompt, &user_message,
        ).await?;

        if let Some(ref identity) = reader_identity {
            context.push(("system".into(), format!(
                "与你对话的读者身份是：{}。", identity
            )));
        }

        context.push(("user".into(), user_message.clone()));

        let response = self.llm.chat_messages(context).await?;

        // 保存消息
        let user_msg = ChatMessage::new(
            user_id, character_id, novel_id,
            "user".into(), user_message,
            reader_identity.clone(), Some(current_chapter),
        );
        let char_msg = ChatMessage::new(
            user_id, character_id, novel_id,
            "character".into(), response.clone(),
            reader_identity, Some(current_chapter),
        );
        self.memory_manager.save_and_consolidate(
            user_msg, char_msg, character_id, user_id, novel_id,
        ).await?;

        Ok(response)
    }

    /// Fetch paginated chat history for a character-user pair.
    pub async fn get_history(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<ChatMessage>> {
        self.memory_manager.chat_repo
            .find_by_character_user(character_id, user_id, limit, offset)
            .await
    }

    /// Fetch memories for a character-user-novel combination, optionally filtered by layer.
    pub async fn get_memories(
        &self,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        layer: crate::domain::entities::memory::MemoryLayer,
    ) -> Result<Vec<Memory>> {
        self.memory_manager.memory_repo
            .find_by_layer(character_id, user_id, novel_id, layer)
            .await
    }

    /// Clear the short-term (Redis) cache for a character-user pair.
    pub async fn clear_short_memory(
        &self,
        character_id: Uuid,
        user_id: Uuid,
    ) -> Result<()> {
        self.memory_manager.cache
            .clear(character_id, user_id)
            .await
    }
}
