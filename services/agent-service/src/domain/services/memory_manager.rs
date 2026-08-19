use anyhow::Result;
use std::sync::Arc;
use tracing::warn;
use uuid::Uuid;

use crate::domain::entities::memory::{ChatMessage, Memory, MemoryLayer};
use crate::domain::ports::{EmbeddingGenerator, MessageCache, TextSummarizer};
use crate::domain::repositories::{ChatRepository, MemoryRepository};

const SHORT_TERM_LIMIT: usize = 10;
const MID_TERM_TRIGGER: usize = 20;
/// Maximum number of semantically similar memories to inject into context.
const SEMANTIC_SEARCH_LIMIT: usize = 5;
const MAX_MEMORY_BLOCK_CHARS: usize = 4_000;
/// pgvector column width (vector(1536)); promotion tolerates any other
/// provider dimension by skipping rather than failing the projection.
const EMBEDDING_DIMS: usize = 1536;
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

/// Outcome of a permanent-memory write so the caller can distinguish a
/// durable save from a SPEC 6.2.4 skip (and whether the skip is retryable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermanentMemorySave {
    Saved,
    /// Embedding generation failed; the caller may retry (transient).
    SkippedEmbeddingUnavailable,
    /// Provider vector is not 1536-dim; retrying is futile (policy).
    SkippedWrongDimensions,
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

        // Long-term producer: promote the committed mid-term summary into the
        // long-term layer so continuity is semantically retrievable across
        // sessions. A promotion happens only with a correctly-dimensioned
        // embedding: generation failure or a non-1536 provider vector skips
        // the promotion (SPEC 6.2.3 requires an embedding on every long
        // entry; an embedding-less row would be unreachable). The Mid record
        // already preserves the summary either way.
        let embedding = match self
            .embedding
            .generate_embedding(memory.content.as_str())
            .await
        {
            Ok(vector) if vector.len() == EMBEDDING_DIMS => Some(vector),
            _ => None,
        };
        let Some(embedding) = embedding else {
            return Ok(());
        };
        let promoted = Memory {
            id: uuid::Uuid::new_v4(),
            character_id,
            user_id,
            novel_id,
            layer: MemoryLayer::Long,
            content: memory.content.clone(),
            importance: memory.importance,
            chapter_number: chapter_context,
            embedding: Some(embedding),
            created_at: chrono::Utc::now(),
        };
        self.memory_repo.save(&promoted).await?;
        Ok(())
    }

    /// 保存永久记忆（重大选择、关系变化）
    /// Generates an embedding for the event text so it can be found via semantic search.
    /// SPEC 6.2.4: an entry is written only with a correctly-dimensioned
    /// embedding; generation failure or a non-1536 provider vector skips the
    /// save entirely (the caller may retry). The memory id is caller-supplied
    /// (narrative's turn id) so the repository upsert is idempotent on replay.
    /// A replay of a completed key returns immediately: no second embedding
    /// call and no re-write (idempotency fast-path, PK existence check).
    #[allow(clippy::too_many_arguments)] // Same shape as MemoryRepository::find_by_layer.
    pub async fn save_permanent_memory(
        &self,
        memory_id: Uuid,
        character_id: Uuid,
        user_id: Uuid,
        novel_id: Uuid,
        chapter_number: i32,
        event: &str,
        importance: i32,
    ) -> Result<PermanentMemorySave> {
        if self.memory_repo.exists(memory_id).await? {
            return Ok(PermanentMemorySave::Saved);
        }
        let embedding = match self.embedding.generate_embedding(event).await {
            Ok(vector) if vector.len() == EMBEDDING_DIMS => vector,
            Ok(_) => {
                warn!(
                    %memory_id,
                    "permanent memory skipped: provider embedding is not {} dims",
                    EMBEDDING_DIMS
                );
                return Ok(PermanentMemorySave::SkippedWrongDimensions);
            }
            Err(error) => {
                warn!(%error, %memory_id, "permanent memory skipped: embedding unavailable");
                return Ok(PermanentMemorySave::SkippedEmbeddingUnavailable);
            }
        };
        let mut memory = Memory::new_permanent(
            character_id,
            user_id,
            novel_id,
            event.to_string(),
            importance,
            chapter_number,
        );
        memory.id = memory_id;
        memory.embedding = Some(embedding);
        self.memory_repo.save(&memory).await?;
        Ok(PermanentMemorySave::Saved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use crate::domain::repositories::{BeginChatTurn, ChatTurnClaim};

    struct RecordingMemoryRepo {
        saved: Mutex<Vec<Memory>>,
    }

    #[async_trait::async_trait]
    impl MemoryRepository for RecordingMemoryRepo {
        async fn exists(&self, id: Uuid) -> Result<bool> {
            Ok(self
                .saved
                .lock()
                .unwrap()
                .iter()
                .any(|memory| memory.id == id))
        }

        async fn save(&self, memory: &Memory) -> Result<()> {
            self.saved.lock().unwrap().push(memory.clone());
            Ok(())
        }

        async fn find_by_layer(
            &self,
            _character_id: Uuid,
            _user_id: Uuid,
            _novel_id: Uuid,
            _layer: MemoryLayer,
            _max_chapter: i32,
            _limit: i64,
            _offset: i64,
        ) -> Result<Vec<Memory>> {
            Ok(vec![])
        }

        async fn search_similar(
            &self,
            _character_id: Uuid,
            _user_id: Uuid,
            _novel_id: Uuid,
            _embedding: &[f32],
            _max_chapter: i32,
            _limit: usize,
        ) -> Result<Vec<Memory>> {
            Ok(vec![])
        }
    }

    struct CountingChatRepo {
        count: usize,
    }

    #[async_trait::async_trait]
    impl ChatRepository for CountingChatRepo {
        async fn begin_turn(&self, _claim: &ChatTurnClaim) -> Result<BeginChatTurn> {
            unreachable!()
        }

        async fn renew_turn(&self, _turn_id: Uuid, _attempt: i64) -> Result<bool> {
            unreachable!()
        }

        async fn complete_turn(
            &self,
            _claim: &ChatTurnClaim,
            _attempt: i64,
            _user_message: &ChatMessage,
            _character_message: &ChatMessage,
        ) -> Result<()> {
            unreachable!()
        }

        async fn fail_turn(
            &self,
            _turn_id: Uuid,
            _attempt: i64,
            _failure_code: &str,
        ) -> Result<bool> {
            unreachable!()
        }

        async fn find_recent(
            &self,
            _character_id: Uuid,
            _user_id: Uuid,
            _novel_id: Uuid,
            _max_chapter: i32,
            _limit: usize,
        ) -> Result<Vec<ChatMessage>> {
            Ok(vec![])
        }

        async fn count(
            &self,
            _character_id: Uuid,
            _user_id: Uuid,
            _novel_id: Uuid,
        ) -> Result<usize> {
            Ok(self.count)
        }

        async fn find_by_character_user(
            &self,
            _character_id: Uuid,
            _user_id: Uuid,
            _novel_id: Uuid,
            _max_chapter: i32,
            _limit: i64,
            _offset: i64,
        ) -> Result<Vec<ChatMessage>> {
            Ok(vec![])
        }
    }

    #[derive(Clone)]
    struct FakeSummarizer(String);

    #[async_trait::async_trait]
    impl TextSummarizer for FakeSummarizer {
        async fn summarize(&self, _system: &str, _text: &str) -> Result<String> {
            Ok(self.0.clone())
        }
    }

    #[derive(Clone)]
    struct FakeEmbedding {
        dims: usize,
        fail: bool,
    }

    #[async_trait::async_trait]
    impl EmbeddingGenerator for FakeEmbedding {
        async fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>> {
            if self.fail {
                anyhow::bail!("embedding unavailable");
            }
            Ok(vec![0.1; self.dims])
        }
    }

    /// Counts provider calls so tests can prove replay makes no new call.
    #[derive(Clone)]
    struct CountingEmbedding {
        dims: usize,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl EmbeddingGenerator for CountingEmbedding {
        async fn generate_embedding(&self, _text: &str) -> Result<Vec<f32>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![0.1; self.dims])
        }
    }

    struct NoopCache;

    #[async_trait::async_trait]
    impl MessageCache for NoopCache {
        async fn get_recent_messages(
            &self,
            _character_id: Uuid,
            _user_id: Uuid,
            _max_chapter: i32,
            _limit: usize,
        ) -> Result<Vec<ChatMessage>> {
            Ok(vec![])
        }

        async fn push_turn(
            &self,
            _character_id: Uuid,
            _user_id: Uuid,
            _user_message: &ChatMessage,
            _character_message: &ChatMessage,
        ) -> Result<bool> {
            Ok(true)
        }

        async fn clear(&self, _character_id: Uuid, _user_id: Uuid) -> Result<()> {
            Ok(())
        }

        async fn clear_user(&self, _user_id: Uuid) -> Result<()> {
            Ok(())
        }

        async fn clear_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> Result<()> {
            Ok(())
        }

        async fn allow_user(&self, _user_id: Uuid) -> Result<()> {
            Ok(())
        }

        async fn allow_novel(&self, _user_id: Uuid, _novel_id: Uuid) -> Result<()> {
            Ok(())
        }
    }

    fn manager(repo: Arc<RecordingMemoryRepo>, embedding: Arc<FakeEmbedding>) -> MemoryManager {
        MemoryManager {
            memory_repo: repo,
            chat_repo: Arc::new(CountingChatRepo { count: 0 }),
            cache: Arc::new(NoopCache),
            llm: Arc::new(FakeSummarizer("压缩后的摘要".into())),
            embedding,
        }
    }

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

    #[tokio::test]
    async fn mid_consolidation_promotes_a_long_term_memory_with_embedding() {
        let repo = Arc::new(RecordingMemoryRepo {
            saved: Mutex::new(vec![]),
        });
        let manager = manager(
            repo.clone(),
            Arc::new(FakeEmbedding {
                dims: 1536,
                fail: false,
            }),
        );
        let (character_id, user_id, novel_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        manager
            .consolidate_to_mid_term(character_id, user_id, novel_id, Some(3))
            .await
            .unwrap();
        let saved = repo.saved.lock().unwrap().clone();
        assert_eq!(
            saved.len(),
            2,
            "expected a Mid record and its Long promotion"
        );
        let mid = saved.iter().find(|m| m.layer == MemoryLayer::Mid).unwrap();
        let long = saved.iter().find(|m| m.layer == MemoryLayer::Long).unwrap();
        assert_eq!(mid.content, long.content);
        assert_eq!(mid.chapter_number, long.chapter_number);
        assert_eq!(mid.importance, long.importance);
        assert_eq!(mid.embedding, None);
        assert_eq!(long.embedding.as_deref().map(|e| e.len()), Some(1536));
    }

    #[tokio::test]
    async fn failed_embedding_skips_the_long_promotion() {
        let repo = Arc::new(RecordingMemoryRepo {
            saved: Mutex::new(vec![]),
        });
        let manager = manager(
            repo.clone(),
            Arc::new(FakeEmbedding {
                dims: 1536,
                fail: true,
            }),
        );
        let (character_id, user_id, novel_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        manager
            .consolidate_to_mid_term(character_id, user_id, novel_id, Some(3))
            .await
            .unwrap();
        let saved = repo.saved.lock().unwrap().clone();
        // SPEC 6.2.3: no embedding-less long entries; the Mid summary alone
        // preserves continuity when embedding generation fails.
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].layer, MemoryLayer::Mid);
        assert_eq!(saved[0].embedding, None);
    }

    #[tokio::test]
    async fn permanent_memory_uses_the_caller_memory_id_and_embeds() {
        let repo = Arc::new(RecordingMemoryRepo {
            saved: Mutex::new(vec![]),
        });
        let manager = manager(
            repo.clone(),
            Arc::new(FakeEmbedding {
                dims: 1536,
                fail: false,
            }),
        );
        let memory_id = Uuid::new_v4();
        manager
            .save_permanent_memory(
                memory_id,
                Uuid::new_v4(),
                Uuid::new_v4(),
                Uuid::new_v4(),
                5,
                "选择了北境之路",
                8,
            )
            .await
            .unwrap();
        let saved = repo.saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].id, memory_id);
        assert_eq!(saved[0].layer, MemoryLayer::Permanent);
        assert_eq!(saved[0].embedding.as_deref().map(|e| e.len()), Some(1536));
    }

    #[tokio::test]
    async fn permanent_memory_skips_save_when_embedding_fails_or_mis_dimensioned() {
        let cases = [
            (1536, true, PermanentMemorySave::SkippedEmbeddingUnavailable),
            (1024, false, PermanentMemorySave::SkippedWrongDimensions),
        ];
        for (dims, fail, expected_outcome) in cases {
            let repo = Arc::new(RecordingMemoryRepo {
                saved: Mutex::new(vec![]),
            });
            let manager = manager(repo.clone(), Arc::new(FakeEmbedding { dims, fail }));
            let outcome = manager
                .save_permanent_memory(
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    5,
                    "选择了北境之路",
                    8,
                )
                .await
                .unwrap();
            // SPEC 6.2.4: no embedding-less permanent rows, and the caller can
            // tell a retryable skip from a policy skip.
            assert!(repo.saved.lock().unwrap().is_empty());
            assert_eq!(outcome, expected_outcome);
        }
    }

    #[tokio::test]
    async fn completed_key_replay_makes_no_embedding_call_and_no_re_save() {
        let repo = Arc::new(RecordingMemoryRepo {
            saved: Mutex::new(vec![]),
        });
        let embedding_calls = Arc::new(AtomicUsize::new(0));
        let manager = MemoryManager {
            memory_repo: repo.clone(),
            chat_repo: Arc::new(CountingChatRepo { count: 0 }),
            cache: Arc::new(NoopCache),
            llm: Arc::new(FakeSummarizer("压缩后的摘要".into())),
            embedding: Arc::new(CountingEmbedding {
                dims: 1536,
                calls: embedding_calls.clone(),
            }),
        };
        let memory_id = Uuid::new_v4();
        let (character_id, user_id, novel_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        // First write: one embedding call, one save.
        manager
            .save_permanent_memory(
                memory_id,
                character_id,
                user_id,
                novel_id,
                5,
                "选择了北境之路",
                8,
            )
            .await
            .unwrap();
        // Replay of the completed key: exists() fast-path, no embedding, no save.
        manager
            .save_permanent_memory(
                memory_id,
                character_id,
                user_id,
                novel_id,
                5,
                "选择了北境之路",
                8,
            )
            .await
            .unwrap();
        assert_eq!(embedding_calls.load(Ordering::SeqCst), 1);
        assert_eq!(repo.saved.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn wrong_dimension_embedding_skips_the_long_promotion() {
        let repo = Arc::new(RecordingMemoryRepo {
            saved: Mutex::new(vec![]),
        });
        // A provider returning a non-1536 vector must not poison the
        // vector(1536) column; promotion is skipped, Mid remains.
        let manager = manager(
            repo.clone(),
            Arc::new(FakeEmbedding {
                dims: 1024,
                fail: false,
            }),
        );
        let (character_id, user_id, novel_id) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        manager
            .consolidate_to_mid_term(character_id, user_id, novel_id, Some(3))
            .await
            .unwrap();
        let saved = repo.saved.lock().unwrap().clone();
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].layer, MemoryLayer::Mid);
    }
}
