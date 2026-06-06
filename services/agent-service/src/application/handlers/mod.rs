use std::sync::Arc;
use anyhow::Result;
use uuid::Uuid;
use futures::{Stream, StreamExt};

use crate::domain::entities::memory::ChatMessage;
use crate::domain::services::memory_manager::MemoryManager;
use crate::domain::repositories::CharacterInfoRepository;
use crate::infrastructure::llm::LlmClient;

pub struct AgentCommandHandler {
    pub memory_manager: Arc<MemoryManager>,
    pub character_repo: Arc<dyn CharacterInfoRepository>,
    pub llm: Arc<LlmClient>,
}

impl AgentCommandHandler {
    /// 流式对话（SSE）
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

        // 构建完整上下文（4层记忆）
        let mut context = self.memory_manager.build_context(
            character_id, user_id, novel_id,
            current_chapter, &system_prompt,
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

        // 保存消息（异步，不阻塞流）
        let mm = self.memory_manager.clone();
        let user_msg = ChatMessage::new(
            user_id, character_id, novel_id,
            "user".into(), user_message,
            reader_identity.clone(), Some(current_chapter),
        );

        // 收集完整响应后保存（通过包装流实现）
        let full_response = Arc::new(tokio::sync::Mutex::new(String::new()));
        let full_response_clone = full_response.clone();

        let wrapped_stream = stream.map(move |chunk| {
            let fr = full_response_clone.clone();
            if let Ok(ref text) = chunk {
                let text = text.clone();
                let fr2 = fr.clone();
                tokio::spawn(async move {
                    fr2.lock().await.push_str(&text);
                });
            }
            chunk
        });

        // 流结束后保存记忆（通过 tokio::spawn 异步处理）
        let mm_clone = mm.clone();
        let reader_identity_clone = reader_identity.clone();
        let full_response_final = full_response.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            let response_text = full_response_final.lock().await.clone();
            if !response_text.is_empty() {
                let char_msg = ChatMessage::new(
                    user_id, character_id, novel_id,
                    "character".into(), response_text,
                    reader_identity_clone, Some(current_chapter),
                );
                let _ = mm_clone.save_and_consolidate(
                    user_msg, char_msg,
                    character_id, user_id, novel_id,
                ).await;
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

        let mut context = self.memory_manager.build_context(
            character_id, user_id, novel_id,
            current_chapter, &system_prompt,
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
}
