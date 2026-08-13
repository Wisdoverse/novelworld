use crate::domain::entities::world_session::WorldEntryContext;
use crate::domain::repositories::{
    ChapterInfo, ChapterReadRepository, NovelInfo, PlayerEntryContext,
};
use crate::domain::services::narrative_transition::CanonContext;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::ports::ReadinessProbe;
pub struct NovelServiceClient {
    client: Client,
    base_url: String,
}

const NOVEL_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);

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
impl ReadinessProbe for NovelServiceClient {
    async fn is_ready(&self) -> bool {
        matches!(
            self.client
                .get(format!("{}/ready", self.base_url))
                .send()
                .await,
            Ok(response) if response.status().is_success()
        )
    }
}

#[derive(serde::Deserialize)]
struct ChapterResponse {
    content: String,
    is_key_node: bool,
    key_node_description: Option<String>,
}

#[derive(serde::Deserialize)]
struct NovelResponse {
    id: Uuid,
    title: String,
    deviation_mode: String,
    world_summary: Option<String>,
}

#[derive(serde::Deserialize)]
struct ReadingProgressResponse {
    reader_identity_type: String,
}

#[async_trait]
impl ChapterReadRepository for NovelServiceClient {
    async fn get_chapter(
        &self,
        novel_id: Uuid,
        chapter_number: i32,
        user_id: Uuid,
    ) -> Result<Option<ChapterInfo>> {
        let url = format!(
            "{}/novels/{}/chapters/{}",
            self.base_url, novel_id, chapter_number
        );
        let resp = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .send()
            .await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(anyhow!("Novel service returned {}", resp.status()));
        }
        let ch: ChapterResponse = resp.json().await?;
        Ok(Some(ChapterInfo {
            content: ch.content,
            is_key_node: ch.is_key_node,
            key_node_description: ch.key_node_description,
        }))
    }

    async fn get_novel_info(&self, novel_id: Uuid, user_id: Uuid) -> Result<Option<NovelInfo>> {
        let url = format!("{}/novels/{}", self.base_url, novel_id);
        let resp = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .send()
            .await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(anyhow!("Novel service returned {}", resp.status()));
        }
        let n: NovelResponse = resp.json().await?;
        Ok(Some(NovelInfo {
            id: n.id,
            title: n.title,
            deviation_mode: n.deviation_mode,
            world_summary: n.world_summary,
        }))
    }

    async fn get_canon_context(
        &self,
        novel_id: Uuid,
        checkpoint_chapter: i32,
        user_id: Uuid,
    ) -> Result<Option<CanonContext>> {
        let url = format!(
            "{}/internal/novels/{}/canon-context/{}",
            self.base_url, novel_id, checkpoint_chapter
        );
        let resp = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .send()
            .await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(anyhow!("Novel service returned {}", resp.status()));
        }
        let context = resp.json::<CanonContext>().await?;
        context
            .validate()
            .map_err(|error| anyhow!("Novel service returned invalid canon context: {error}"))?;
        Ok(Some(context))
    }

    async fn get_player_entry_context(
        &self,
        novel_id: Uuid,
        user_id: Uuid,
        checkpoint_chapter: Option<i32>,
        proposed_name: Option<&str>,
    ) -> Result<Option<PlayerEntryContext>> {
        let url = format!(
            "{}/internal/novels/{}/player-entry",
            self.base_url, novel_id
        );
        let resp = self
            .client
            .post(&url)
            .header("X-User-Id", user_id.to_string())
            .json(&serde_json::json!({
                "checkpoint_chapter": checkpoint_chapter,
                "proposed_name": proposed_name,
            }))
            .send()
            .await?;
        if resp.status().as_u16() == 404 {
            return Ok(None);
        }
        if !resp.status().is_success() {
            return Err(anyhow!("Novel service returned {}", resp.status()));
        }
        let context = resp.json::<PlayerEntryContext>().await?;
        if context.checkpoint_chapter < 1
            || context.locations.len() > 256
            || context.locations.iter().any(|location| {
                location.id.trim() != location.id
                    || location.id.is_empty()
                    || location.id.chars().count() > 200
                    || location.id.chars().any(char::is_control)
                    || location.name.trim().is_empty()
                    || location.name.chars().count() > 1_000
            })
        {
            return Err(anyhow!(
                "Novel service returned invalid player entry context"
            ));
        }
        Ok(Some(context))
    }

    async fn uses_original_player_identity(&self, novel_id: Uuid, user_id: Uuid) -> Result<bool> {
        let resp = self
            .client
            .get(format!("{}/progress/{}", self.base_url, novel_id))
            .header("X-User-Id", user_id.to_string())
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Novel service returned {}", resp.status()));
        }
        match resp
            .json::<ReadingProgressResponse>()
            .await?
            .reader_identity_type
            .as_str()
        {
            "self" => Ok(true),
            "character" => Ok(false),
            value => Err(anyhow!(
                "Novel service returned invalid reader identity type {value}"
            )),
        }
    }

    async fn get_world_entry_context(
        &self,
        novel_id: Uuid,
        checkpoint_chapter: i32,
        user_id: Uuid,
    ) -> Result<Option<WorldEntryContext>> {
        let url = format!(
            "{}/internal/novels/{}/world-entry/{}",
            self.base_url, novel_id, checkpoint_chapter
        );
        let response = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(anyhow!(
                "Novel service returned {} for world entry",
                response.status()
            ));
        }
        let context = response.json::<WorldEntryContext>().await?;
        context
            .validate()
            .map_err(|error| anyhow!("Novel service returned invalid world entry: {error}"))?;
        if context.checkpoint_chapter != checkpoint_chapter {
            return Err(anyhow!("Novel service returned the wrong world checkpoint"));
        }
        Ok(Some(context))
    }
}
