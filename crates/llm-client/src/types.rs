use serde::{Deserialize, Serialize};

#[derive(Debug)]
pub struct LlmApiError {
    pub status: u16,
    pub message: String,
}

impl std::fmt::Display for LlmApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LLM API error {}: {}", self.status, self.message)
    }
}
impl std::error::Error for LlmApiError {}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".into(), content: content.into() }
    }
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".into(), content: content.into() }
    }
    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".into(), content: content.into() }
    }
}

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub model: String,
    pub messages: Vec<ChatMessage>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub stream: bool,
    pub json_mode: bool,
}

impl ChatRequest {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            messages: vec![],
            temperature: None,
            max_tokens: None,
            stream: false,
            json_mode: false,
        }
    }

    pub fn message(mut self, role: &str, content: impl Into<String>) -> Self {
        self.messages.push(ChatMessage { role: role.into(), content: content.into() });
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatResponse {
    pub content: String,
    pub model: String,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
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
        Self { provider: Provider::OpenAI, api_key: api_key.into(), base_url: None }
    }
    pub fn anthropic(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::Anthropic, api_key: api_key.into(), base_url: None }
    }
    pub fn gemini(api_key: impl Into<String>) -> Self {
        Self { provider: Provider::Gemini, api_key: api_key.into(), base_url: None }
    }
    pub fn openai_compatible(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self { provider: Provider::OpenAICompatible, api_key: api_key.into(), base_url: Some(base_url.into()) }
    }
}
