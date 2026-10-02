use uuid::Uuid;

use crate::domain::value_objects::DeviationMode;

/// Import-novel command.
#[derive(Debug)]
pub struct ImportNovelCommand {
    pub user_id: Uuid,
    pub title: String,
    pub author: Option<String>,
    /// Raw text content supplied by pasting.
    pub raw_content: Option<String>,
    /// Raw bytes from an uploaded file; omitted for pasted imports.
    pub source_bytes: Option<bytes::Bytes>,
    pub deviation_mode: Option<DeviationMode>,
}
