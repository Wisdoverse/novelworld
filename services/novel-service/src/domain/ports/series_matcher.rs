use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_SERIES_MATCH_CANDIDATES: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesBookMetadata {
    pub title: String,
    pub author: Option<String>,
    pub genre: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesMatchCandidate {
    pub series_id: Option<Uuid>,
    pub source_novel_id: Uuid,
    pub name: String,
    pub book: SeriesBookMetadata,
}

/// Read-only classification; a suggestion never associates books or creates rules.
#[async_trait]
pub trait SeriesMatcherPort: Send + Sync {
    async fn suggest(
        &self,
        target: &SeriesBookMetadata,
        candidates: &[SeriesMatchCandidate],
    ) -> Result<Option<usize>>;
}
