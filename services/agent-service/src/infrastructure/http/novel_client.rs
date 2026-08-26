use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::domain::ports::{
    LoreContextPort, LoreExcerpt, ReadinessProbe, ReadingContext, ReadingContextPort,
};
use crate::domain::repositories::{CharacterInfo, CharacterInfoRepository};

const NOVEL_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);

/// HTTP adapter that fetches character info from novel-service,
/// replacing the previous direct-DB query against novel-service's tables.
pub struct NovelServiceClient {
    client: Client,
    base_url: String,
}

/// Minimal deserialization type for the character data used at chat time.
/// Persona fields mirror the novel-service Character entity's public shape.
#[derive(Debug, Deserialize)]
struct CharacterResponse {
    id: Uuid,
    name: String,
    novel_id: Uuid,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    personality: Option<String>,
    #[serde(default)]
    background: Option<String>,
    #[serde(default)]
    speaking_style: Option<String>,
    #[serde(default)]
    persona_source_chapter_high_water: Option<i32>,
    #[serde(default)]
    first_appearance_chapter: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct ReadingProgressResponse {
    user_id: Uuid,
    novel_id: Uuid,
    current_chapter: i32,
    reader_identity: Option<String>,
    reader_identity_type: String,
    reader_character_id: Option<Uuid>,
    deviation_mode: String,
}

#[derive(Debug, Serialize)]
struct LoreSearchRequest<'a> {
    query: &'a str,
    max_chapter: i32,
    limit: usize,
}

#[derive(Debug, Deserialize)]
struct LoreSearchResponse {
    excerpts: Vec<LoreExcerpt>,
}

impl NovelServiceClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(NOVEL_SERVICE_TIMEOUT)
                .build()
                .expect("valid novel-service HTTP client configuration"),
            base_url,
        }
    }
}

#[async_trait]
impl CharacterInfoRepository for NovelServiceClient {
    async fn find_by_id(&self, id: Uuid, user_id: Uuid) -> Result<Option<CharacterInfo>> {
        // Fetch character from novel-service API
        let url = format!("{}/characters/{}", self.base_url, id);
        let resp = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .send()
            .await
            .map_err(|e| anyhow!("Failed to reach novel-service at {}: {}", url, e))?;

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            return Err(anyhow!(
                "novel-service returned {} for character {}",
                resp.status(),
                id
            ));
        }

        let ch: CharacterResponse = resp.json().await?;

        Ok(Some(CharacterInfo {
            id: ch.id,
            name: ch.name,
            novel_id: ch.novel_id,
            aliases: ch.aliases,
            role: ch.role,
            description: ch.description,
            personality: ch.personality,
            background: ch.background,
            speaking_style: ch.speaking_style,
            persona_source_chapter_high_water: ch.persona_source_chapter_high_water,
            first_appearance_chapter: ch.first_appearance_chapter,
        }))
    }
}

#[async_trait]
impl ReadingContextPort for NovelServiceClient {
    async fn find(&self, novel_id: Uuid, user_id: Uuid) -> Result<Option<ReadingContext>> {
        let url = format!("{}/progress/{}", self.base_url, novel_id);
        let response = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .send()
            .await
            .map_err(|error| anyhow!("Failed to reach novel-service at {}: {}", url, error))?;

        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(anyhow!(
                "novel-service returned {} for reading progress {}",
                response.status(),
                novel_id
            ));
        }

        let progress: ReadingProgressResponse = response.json().await?;
        if progress.user_id != user_id
            || progress.novel_id != novel_id
            || progress.current_chapter < 1
            || !matches!(progress.reader_identity_type.as_str(), "self" | "character")
            || !matches!(
                progress.deviation_mode.as_str(),
                "canon" | "creative" | "remix"
            )
            || progress.reader_identity.as_deref().is_some_and(|identity| {
                identity.chars().count() > 200 || identity.chars().any(char::is_control)
            })
            || !matches!(
                (
                    progress.reader_identity_type.as_str(),
                    progress.reader_identity.as_ref(),
                    progress.reader_character_id,
                ),
                ("self", _, None) | ("character", Some(_), Some(_))
            )
        {
            return Err(anyhow!("novel-service returned invalid reading context"));
        }

        Ok(Some(ReadingContext {
            user_id: progress.user_id,
            novel_id: progress.novel_id,
            current_chapter: progress.current_chapter,
            reader_identity: progress.reader_identity,
            reader_identity_type: progress.reader_identity_type,
            reader_character_id: progress.reader_character_id,
            deviation_mode: progress.deviation_mode,
        }))
    }
}

#[async_trait]
impl LoreContextPort for NovelServiceClient {
    async fn search(
        &self,
        novel_id: Uuid,
        user_id: Uuid,
        max_chapter: i32,
        query: &str,
        limit: usize,
    ) -> Result<Vec<LoreExcerpt>> {
        let url = format!("{}/novels/{}/lore/search", self.base_url, novel_id);
        let response = self
            .client
            .post(&url)
            .header("X-User-Id", user_id.to_string())
            .json(&LoreSearchRequest {
                query,
                max_chapter,
                limit,
            })
            .send()
            .await
            .map_err(|error| anyhow!("Failed to reach novel-service at {}: {}", url, error))?;

        if !response.status().is_success() {
            return Err(anyhow!(
                "novel-service returned {} for lore search {}",
                response.status(),
                novel_id
            ));
        }

        let body: LoreSearchResponse = response.json().await?;
        if body.excerpts.iter().any(|excerpt| {
            excerpt.chapter_number < 1
                || excerpt.chapter_number > max_chapter
                || excerpt.content.trim().is_empty()
        }) {
            return Err(anyhow!("novel-service returned invalid lore context"));
        }
        Ok(body.excerpts)
    }
}

#[async_trait]
impl ReadinessProbe for NovelServiceClient {
    async fn is_ready(&self) -> bool {
        self.client
            .get(format!("{}/ready", self.base_url))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }
}
