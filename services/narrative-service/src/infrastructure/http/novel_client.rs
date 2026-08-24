use crate::domain::entities::{game_rules::GameRuleTemplate, world_session::WorldEntryContext};
use crate::domain::repositories::{
    ChapterInfo, ChapterReadRepository, CharacterBrief, GameRuleTemplateRequestError, NovelInfo,
    PlayerEntryContext,
};
use crate::domain::services::narrative_transition::CanonContext;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use reqwest::header::RETRY_AFTER;
use reqwest::Client;
use std::time::Duration;
use uuid::Uuid;

use crate::domain::ports::ReadinessProbe;
pub struct NovelServiceClient {
    client: Client,
    base_url: String,
    internal_service_token: String,
}

const NOVEL_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);
const GAME_RULE_REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

impl NovelServiceClient {
    pub fn new(base_url: String, internal_service_token: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(NOVEL_SERVICE_TIMEOUT)
                .build()
                .expect("valid novel-service HTTP client configuration"),
            base_url,
            internal_service_token,
        }
    }

    async fn reading_progress(
        &self,
        novel_id: Uuid,
        user_id: Uuid,
    ) -> Result<ReadingProgressResponse> {
        let response = self
            .client
            .get(format!("{}/progress/{}", self.base_url, novel_id))
            .header("X-User-Id", user_id.to_string())
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!("Novel service returned {}", response.status()));
        }
        response
            .json::<ReadingProgressResponse>()
            .await
            .map_err(Into::into)
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
    current_chapter: i32,
    reader_identity_type: String,
}

#[derive(serde::Deserialize)]
struct CharacterListRow {
    id: Uuid,
    #[serde(default)]
    role: String,
    #[serde(default)]
    first_appearance_chapter: Option<i32>,
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

    async fn list_characters(&self, novel_id: Uuid, user_id: Uuid) -> Result<Vec<CharacterBrief>> {
        let url = format!("{}/novels/{}/characters", self.base_url, novel_id);
        let resp = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(anyhow!("Novel service returned {}", resp.status()));
        }
        let rows: Vec<CharacterListRow> = resp.json().await?;
        Ok(rows
            .into_iter()
            .map(|row| CharacterBrief {
                id: row.id,
                role: row.role,
                first_appearance_chapter: row.first_appearance_chapter,
            })
            .collect())
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
            .header("X-Internal-Service-Token", &self.internal_service_token)
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

    async fn get_current_chapter(&self, novel_id: Uuid, user_id: Uuid) -> Result<i32> {
        let progress = self.reading_progress(novel_id, user_id).await?;
        if progress.current_chapter < 1 {
            return Err(anyhow!("Novel service returned an invalid current chapter"));
        }
        Ok(progress.current_chapter)
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
            .header("X-Internal-Service-Token", &self.internal_service_token)
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

    async fn reader_identity_is_self(&self, novel_id: Uuid, user_id: Uuid) -> Result<bool> {
        match self
            .reading_progress(novel_id, user_id)
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
            .header("X-Internal-Service-Token", &self.internal_service_token)
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

    async fn request_game_rule_template(
        &self,
        novel_id: Uuid,
        user_id: Uuid,
    ) -> std::result::Result<GameRuleTemplate, GameRuleTemplateRequestError> {
        let response = self
            .client
            .post(format!(
                "{}/internal/novels/{}/game-rules",
                self.base_url, novel_id
            ))
            .timeout(GAME_RULE_REQUEST_TIMEOUT)
            .header("X-User-Id", user_id.to_string())
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .send()
            .await
            .map_err(|error| GameRuleTemplateRequestError::Unavailable(error.into()))?;
        if !response.status().is_success() {
            let status = response.status();
            let retry_after_seconds = response
                .headers()
                .get(RETRY_AFTER)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.parse::<u64>().ok())
                .filter(|seconds| *seconds > 0);
            if status == reqwest::StatusCode::CONFLICT {
                if let Some(retry_after_seconds) = retry_after_seconds {
                    return Err(GameRuleTemplateRequestError::InProgress {
                        retry_after_seconds,
                    });
                }
            }
            let body = response.json::<serde_json::Value>().await.ok();
            let code = body
                .as_ref()
                .and_then(|body| body.get("error"))
                .and_then(|error| error.get("code"))
                .and_then(serde_json::Value::as_str);
            if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY
                && code == Some("game_rule_generation_exhausted")
            {
                return Err(GameRuleTemplateRequestError::Exhausted);
            }
            return Err(GameRuleTemplateRequestError::Unavailable(anyhow!(
                "Novel service returned {status} for game rules"
            )));
        }
        let template = response
            .json::<GameRuleTemplate>()
            .await
            .map_err(|error| GameRuleTemplateRequestError::Unavailable(error.into()))?;
        template.validate().map_err(|error| {
            GameRuleTemplateRequestError::Unavailable(anyhow!(
                "Novel service returned invalid game rules: {error}"
            ))
        })?;
        if template.novel_id != novel_id {
            return Err(GameRuleTemplateRequestError::Unavailable(anyhow!(
                "Novel service returned game rules for another novel"
            )));
        }
        Ok(template)
    }

    async fn get_game_rule_template(
        &self,
        novel_id: Uuid,
        canon_model_version: i32,
        user_id: Uuid,
    ) -> Result<Option<GameRuleTemplate>> {
        let response = self
            .client
            .get(format!(
                "{}/internal/novels/{}/game-rules/{}",
                self.base_url, novel_id, canon_model_version
            ))
            .header("X-User-Id", user_id.to_string())
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .send()
            .await?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(anyhow!(
                "Novel service returned {} for game rules",
                response.status()
            ));
        }
        let template = response.json::<GameRuleTemplate>().await?;
        template
            .validate()
            .map_err(|error| anyhow!("Novel service returned invalid game rules: {error}"))?;
        if template.novel_id != novel_id || template.canon_model_version != canon_model_version {
            return Err(anyhow!(
                "Novel service returned the wrong game rule template"
            ));
        }
        Ok(Some(template))
    }
}
