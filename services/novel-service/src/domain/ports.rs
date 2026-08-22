use anyhow::Result;
use async_trait::async_trait;
use std::pin::Pin;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug)]
pub struct AccountExportRecord {
    pub kind: String,
    pub data: serde_json::Value,
}

pub type AccountExportStream =
    Pin<Box<dyn futures::Stream<Item = Result<AccountExportRecord>> + Send>>;

pub trait AccountExportPort: Send + Sync {
    fn export_user(&self, user_id: Uuid) -> AccountExportStream;
}

#[async_trait]
pub trait LlmPort: Send + Sync {
    async fn chat_json(&self, task: NovelLlmTask, prompt: &str) -> Result<String>;
}

#[derive(Debug, Clone, Copy)]
pub enum NovelLlmTask {
    ChapterBoundaryDetection,
    CharacterExtraction,
    CanonExtraction,
    NarrativeNodeDetection,
}

#[async_trait]
pub trait ImagePort: Send + Sync {
    async fn generate(&self, prompt: &str) -> Result<String>;
}

#[async_trait]
pub trait ReadinessProbe: Send + Sync {
    async fn is_ready(&self) -> bool;
}

#[async_trait]
pub trait PrivacyCleanupPort: Send + Sync {
    async fn clear_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()>;
    async fn allow_novel(&self, user_id: Uuid, novel_id: Uuid) -> Result<()>;
}

#[async_trait]
pub trait SourceFileStorage: Send + Sync {
    async fn put(&self, key: &str, data: bytes::Bytes) -> Result<()>;
    /// Returns `Ok(None)` when the object does not exist. Any read or size
    /// failure is an `Err`; callers treat absence and failure differently.
    async fn get(&self, key: &str) -> Result<Option<bytes::Bytes>>;
    async fn delete(&self, key: &str) -> Result<()>;
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
