use anyhow::Result;
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatStreamEvent {
    Delta(String),
    Usage(Usage),
    Finished,
}

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatStreamEvent>> + Send>>;

#[derive(Debug)]
pub struct LlmApiError {
    pub status: u16,
    pub message: String,
    pub retry_after: Option<String>,
}

impl std::fmt::Display for LlmApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LLM API error {}: {}", self.status, self.message)
    }
}
impl std::error::Error for LlmApiError {}

#[derive(Debug)]
pub(crate) struct JsonModeEmpty(pub(crate) Option<Usage>);

impl std::fmt::Display for JsonModeEmpty {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("JSON mode returned empty content")
    }
}

impl std::error::Error for JsonModeEmpty {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: "system".into(),
            content: content.into(),
        }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: "user".into(),
            content: content.into(),
        }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: "assistant".into(),
            content: content.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub operation: LlmOperation,
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub json_mode: bool,
    pub thinking: Option<bool>,
}

impl ChatRequest {
    pub fn new(operation: LlmOperation, model: impl Into<String>) -> Self {
        Self {
            operation,
            model: model.into(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stream: false,
            json_mode: false,
            thinking: None,
        }
    }

    pub fn message(mut self, role: &str, content: impl Into<String>) -> Self {
        self.messages.push(ChatMessage {
            role: role.into(),
            content: content.into(),
        });
        self
    }

    pub fn messages(mut self, msgs: Vec<ChatMessage>) -> Self {
        self.messages = msgs;
        self
    }

    pub fn temperature(mut self, t: f32) -> Self {
        self.temperature = Some(t);
        self
    }

    pub fn max_tokens(mut self, n: u32) -> Self {
        self.max_tokens = Some(n);
        self
    }

    pub fn json(mut self) -> Self {
        self.json_mode = true;
        self
    }

    pub fn thinking(mut self, enabled: bool) -> Self {
        self.thinking = Some(enabled);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmOperation {
    SetupConnection,
    ChapterBoundaryDetection,
    CharacterExtraction,
    CanonExtraction,
    NarrativeNodeDetection,
    BranchGeneration,
    NarrativeTransition,
    PlayerChapter,
    CharacterChat,
    MemorySummary,
    OfflineEvaluation,
}

impl LlmOperation {
    pub const ALL: [Self; 11] = [
        Self::SetupConnection,
        Self::ChapterBoundaryDetection,
        Self::CharacterExtraction,
        Self::CanonExtraction,
        Self::NarrativeNodeDetection,
        Self::BranchGeneration,
        Self::NarrativeTransition,
        Self::PlayerChapter,
        Self::CharacterChat,
        Self::MemorySummary,
        Self::OfflineEvaluation,
    ];

    pub const fn to_str(self) -> &'static str {
        match self {
            Self::SetupConnection => "setup_connection",
            Self::ChapterBoundaryDetection => "chapter_boundary_detection",
            Self::CharacterExtraction => "character_extraction",
            Self::CanonExtraction => "canon_extraction",
            Self::NarrativeNodeDetection => "narrative_node_detection",
            Self::BranchGeneration => "branch_generation",
            Self::NarrativeTransition => "narrative_transition",
            Self::PlayerChapter => "player_chapter",
            Self::CharacterChat => "character_chat",
            Self::MemorySummary => "memory_summary",
            Self::OfflineEvaluation => "offline_evaluation",
        }
    }

    pub const fn max_output_tokens(self) -> u32 {
        match self {
            Self::SetupConnection => 8,
            Self::MemorySummary => 256,
            Self::OfflineEvaluation => 800,
            Self::ChapterBoundaryDetection => 2_048,
            Self::CharacterChat => 5_120,
            Self::CanonExtraction => 8_192,
            Self::CharacterExtraction
            | Self::NarrativeNodeDetection
            | Self::BranchGeneration
            | Self::NarrativeTransition => 4_096,
            Self::PlayerChapter => 8_192,
        }
    }
}

impl ChatRequest {
    pub(crate) fn effective_max_output_tokens(&self) -> Option<u32> {
        self.max_tokens.map(|limit| {
            if self.thinking == Some(true) {
                limit.saturating_add(4_096).min(8_192)
            } else {
                limit
            }
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_tokens: Option<u32>,
}

impl Usage {
    pub fn new(
        input_tokens: u32,
        output_tokens: u32,
        cached_input_tokens: Option<u32>,
    ) -> Result<Self> {
        if cached_input_tokens.is_some_and(|cached| cached > input_tokens) {
            anyhow::bail!("cached input tokens exceed total input tokens");
        }
        Ok(Self {
            input_tokens,
            output_tokens,
            cached_input_tokens,
        })
    }
}

#[derive(Debug, Clone)]
pub struct EmbeddingRequest {
    pub model: String,
    pub input: String,
}

#[derive(Debug, Clone)]
pub struct EmbeddingResponse {
    pub embedding: Vec<f32>,
    pub model: String,
}

#[derive(Debug, Clone)]
pub enum Provider {
    OpenAI,
    Anthropic,
    Gemini,
    OpenAICompatible,
}

#[derive(Debug, Clone)]
pub struct ProviderConfig {
    pub provider: Provider,
    pub api_key: String,
    pub base_url: Option<String>,
}

impl ProviderConfig {
    pub fn openai(api_key: impl Into<String>) -> Self {
        Self {
            provider: Provider::OpenAI,
            api_key: api_key.into(),
            base_url: None,
        }
    }
    pub fn anthropic(api_key: impl Into<String>) -> Self {
        Self {
            provider: Provider::Anthropic,
            api_key: api_key.into(),
            base_url: None,
        }
    }
    pub fn gemini(api_key: impl Into<String>) -> Self {
        Self {
            provider: Provider::Gemini,
            api_key: api_key.into(),
            base_url: None,
        }
    }
    pub fn openai_compatible(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            provider: Provider::OpenAICompatible,
            api_key: api_key.into(),
            base_url: Some(base_url.into()),
        }
    }
}
