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
    /// Names attested entirely in chapter 1; never serialized in public responses.
    #[serde(skip)]
    pub world_entities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesMatchCandidate {
    pub series_id: Option<Uuid>,
    pub source_novel_id: Uuid,
    pub name: String,
    pub book: SeriesBookMetadata,
    #[serde(skip)]
    pub member_titles: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesMatchMethod {
    Laya,
    Deepseek,
}
impl SeriesMatchMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Laya => "laya",
            Self::Deepseek => "deepseek",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesSuggestionStatus {
    Suggested,
    Unconfigured,
    Uncertain,
    Unavailable,
    InProgress,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SeriesMatchReason {
    NoCandidates,
    LowConfidence,
    NotConfigured,
    UnsupportedProvider,
    Unavailable,
    InProgress,
    Suggested,
    UnknownOutcome,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeriesSuggestion {
    pub status: SeriesSuggestionStatus,
    pub suggestion: Option<SeriesMatchCandidate>,
    pub method: SeriesMatchMethod,
    pub reason: SeriesMatchReason,
    pub cached: bool,
}

pub enum BeginSeriesMatch {
    Acquired { token: Uuid },
    Cached(SeriesSuggestion),
    InProgress,
    UnknownOutcome,
}

/// Provider projection is separate from the public API and excludes scope IDs.
pub fn provider_book(book: &SeriesBookMetadata) -> serde_json::Value {
    serde_json::json!({"title":book.title,"author":book.author,"genre":book.genre,"chapter_one_entities":book.world_entities})
}

pub fn provider_candidates(candidates: &[SeriesMatchCandidate]) -> Vec<serde_json::Value> {
    candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            serde_json::json!({
                "choice": index, "series_name": candidate.name,
                "book": provider_book(&candidate.book), "member_titles": candidate.member_titles
            })
        })
        .collect()
}

/// Read-only classification; a suggestion never associates books or creates rules.
#[async_trait]
pub trait SeriesMatcherPort: Send + Sync {
    fn identity(&self) -> &str {
        "laya/multilingual"
    }
    async fn suggest(
        &self,
        target: &SeriesBookMetadata,
        candidates: &[SeriesMatchCandidate],
    ) -> Result<Option<usize>>;
}
