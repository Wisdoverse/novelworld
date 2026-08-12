use anyhow::Result;
use async_trait::async_trait;
use thiserror::Error;

#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn chat(&self, system: &str, user: &str) -> Result<String>;
    async fn chat_json(&self, prompt: &str) -> Result<String>;
}

#[async_trait]
pub trait ImagePort: Send + Sync {
    async fn generate(&self, prompt: &str) -> Result<String>;
}

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
    async fn is_ready(&self) -> bool;
}

#[derive(Debug, Error)]
pub enum DocumentExtractionError {
    #[error("unsupported file type; upload a TXT, EPUB, or PDF file")]
    UnsupportedType,
    #[error("{format} file exceeds the {max_bytes} byte upload limit")]
    UploadTooLarge {
        format: &'static str,
        max_bytes: usize,
    },
    #[error("extracted document text exceeds the {max_bytes} byte limit")]
    ExtractedTextTooLarge { max_bytes: usize },
    #[error("the text file encoding is not supported")]
    InvalidTextEncoding,
    #[error("invalid EPUB: {0}")]
    InvalidEpub(String),
    #[error("failed to extract PDF text: {0}")]
    InvalidPdf(String),
    #[error("the document contains no readable text")]
    EmptyDocument,
}

pub trait DocumentTextExtractor: Send + Sync {
    fn extract_text(
        &self,
        file_name: Option<&str>,
        content_type: Option<&str>,
        data: &[u8],
    ) -> std::result::Result<String, DocumentExtractionError>;
}
