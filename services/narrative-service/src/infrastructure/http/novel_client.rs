use crate::domain::entities::{
    game_rules::{
        GameRuleTemplate, SeriesRuleBinding, BASIC_GAME_RULE_PROMPT_VERSION,
        GAME_RULE_PROMPT_VERSION, SERIES_GAME_RULE_PROMPT_VERSION,
    },
    world_session::WorldEntryContext,
};
use crate::domain::repositories::{
    ChapterInfo, ChapterReadRepository, CharacterBrief, GameRuleTemplateRequestError, NovelInfo,
    PlayerEntryContext, ReadingProgressSnapshot,
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

fn supported_game_rule_prompt_version(version: &str) -> bool {
    version == GAME_RULE_PROMPT_VERSION || version == BASIC_GAME_RULE_PROMPT_VERSION
}

impl NovelServiceClient {
    pub fn new(base_url: String, internal_service_token: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(NOVEL_SERVICE_TIMEOUT)
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
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
    ) -> Result<ReadingProgressSnapshot> {
        let response = self
            .client
            .get(format!("{}/progress/{}", self.base_url, novel_id))
            .header("X-User-Id", user_id.to_string())
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(anyhow!("Novel service returned {}", response.status()));
        }
        let progress = response
            .json::<ReadingProgressResponse>()
            .await
            .map_err(anyhow::Error::from)?;
        validate_reading_progress(progress, novel_id, user_id)
    }
}

fn validate_reading_progress(
    progress: ReadingProgressResponse,
    expected_novel_id: Uuid,
    expected_user_id: Uuid,
) -> Result<ReadingProgressSnapshot> {
    if progress.novel_id != expected_novel_id {
        return Err(anyhow!(
            "Novel service returned reading progress for another novel"
        ));
    }
    if progress.user_id != expected_user_id {
        return Err(anyhow!(
            "Novel service returned reading progress for another user"
        ));
    }
    if progress.current_chapter < 1 {
        return Err(anyhow!("Novel service returned an invalid current chapter"));
    }
    let reader_identity_is_self = match progress.reader_identity_type.as_str() {
        "self" => true,
        "character" => false,
        value => {
            return Err(anyhow!(
                "Novel service returned invalid reader identity type {value}"
            ))
        }
    };
    Ok(ReadingProgressSnapshot {
        current_chapter: progress.current_chapter,
        reader_identity_is_self,
    })
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
    user_id: Uuid,
    novel_id: Uuid,
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
        Ok(self
            .reading_progress(novel_id, user_id)
            .await?
            .current_chapter)
    }

    async fn get_reading_progress(
        &self,
        novel_id: Uuid,
        user_id: Uuid,
    ) -> Result<ReadingProgressSnapshot> {
        self.reading_progress(novel_id, user_id).await
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
        Ok(self
            .reading_progress(novel_id, user_id)
            .await?
            .reader_identity_is_self)
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
        prompt_version: &str,
    ) -> std::result::Result<GameRuleTemplate, GameRuleTemplateRequestError> {
        if !supported_game_rule_prompt_version(prompt_version) {
            return Err(GameRuleTemplateRequestError::Unavailable(anyhow!(
                "Unsupported game rule prompt version"
            )));
        }
        let response = self
            .client
            .post(format!(
                "{}/internal/novels/{}/game-rules?prompt_version={}&prefer_series={}",
                self.base_url,
                novel_id,
                prompt_version,
                prompt_version == BASIC_GAME_RULE_PROMPT_VERSION
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
            if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY
                && code == Some("game_rules_unavailable_at_progress")
            {
                return Err(GameRuleTemplateRequestError::UnavailableAtProgress);
            }
            if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY
                && code == Some("game_rule_sources_unavailable")
            {
                return Err(GameRuleTemplateRequestError::SourcesUnavailable);
            }
            if status == reqwest::StatusCode::CONFLICT && code == Some("canon_unavailable") {
                return Err(GameRuleTemplateRequestError::CanonUnavailable);
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
        let expected_prompt =
            if prompt_version == BASIC_GAME_RULE_PROMPT_VERSION && template.series.is_some() {
                SERIES_GAME_RULE_PROMPT_VERSION
            } else {
                prompt_version
            };
        if !template.applies_to_novel(novel_id) || template.prompt_version != expected_prompt {
            return Err(GameRuleTemplateRequestError::Unavailable(anyhow!(
                "Novel service returned the wrong game rule template"
            )));
        }
        Ok(template)
    }

    async fn get_game_rule_template(
        &self,
        novel_id: Uuid,
        canon_model_version: i32,
        user_id: Uuid,
        prompt_version: &str,
        series_binding: Option<&SeriesRuleBinding>,
        require_current_series: bool,
    ) -> Result<Option<GameRuleTemplate>> {
        if !(supported_game_rule_prompt_version(prompt_version)
            || prompt_version == SERIES_GAME_RULE_PROMPT_VERSION)
            || (prompt_version == SERIES_GAME_RULE_PROMPT_VERSION) != series_binding.is_some()
        {
            return Err(anyhow!("Unsupported game rule prompt version"));
        }
        if let Some(binding) = series_binding {
            binding
                .validate()
                .map_err(|error| anyhow!("Invalid series binding: {error}"))?;
        }
        let mut url = reqwest::Url::parse(&format!(
            "{}/internal/novels/{}/game-rules/{}",
            self.base_url, novel_id, canon_model_version
        ))?;
        url.query_pairs_mut()
            .append_pair("prompt_version", prompt_version);
        if let Some(binding) = series_binding {
            url.query_pairs_mut()
                .append_pair("series_id", &binding.series_id.to_string())
                .append_pair("series_revision", &binding.revision.to_string())
                .append_pair(
                    "require_current_series",
                    &require_current_series.to_string(),
                );
        }
        let response = self
            .client
            .get(url)
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
        if !template.applies_to_novel(novel_id)
            || template.binding() != series_binding
            || template.canon_model_version != canon_model_version
            || template.prompt_version != prompt_version
        {
            return Err(anyhow!(
                "Novel service returned the wrong game rule template"
            ));
        }
        Ok(Some(template))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::entities::{
        game_rules::{GameActionRule, GameAttribute, BASIC_ACTION_DESCRIPTION},
        world_session::WorldActionKind,
    };
    use std::{
        collections::HashMap,
        sync::{Arc, Mutex},
    };

    const USER_ID: Uuid = Uuid::from_u128(1);
    const NOVEL_ID: Uuid = Uuid::from_u128(2);

    fn game_rule_template(prompt_version: &str) -> GameRuleTemplate {
        let attributes = ["vigor", "insight", "influence"]
            .into_iter()
            .map(|key| {
                let (label, description) = if prompt_version == BASIC_GAME_RULE_PROMPT_VERSION {
                    crate::domain::entities::game_rules::basic_attribute(key).unwrap()
                } else {
                    (key, "v1 narrative attribute")
                };
                GameAttribute {
                    key: key.into(),
                    label: label.into(),
                    description: description.into(),
                    default_score: 10,
                    source_chapters: vec![2],
                }
            })
            .collect::<Vec<_>>();
        let actions = [
            WorldActionKind::Travel,
            WorldActionKind::Investigate,
            WorldActionKind::Converse,
            WorldActionKind::Ally,
            WorldActionKind::Oppose,
            WorldActionKind::AdvanceThread,
            WorldActionKind::ResolveThread,
            WorldActionKind::PursueGoal,
        ]
        .into_iter()
        .enumerate()
        .map(|(index, kind)| GameActionRule {
            kind,
            attribute_key: attributes[index % attributes.len()].key.clone(),
            difficulty_class: 13,
            description: if prompt_version == BASIC_GAME_RULE_PROMPT_VERSION {
                BASIC_ACTION_DESCRIPTION.into()
            } else {
                "v1 narrative action".into()
            },
            source_chapters: vec![2],
        })
        .collect();
        GameRuleTemplate {
            series: None,
            novel_id: NOVEL_ID,
            canon_model_version: 1,
            schema_version: 1,
            prompt_version: prompt_version.into(),
            minimum_score: 8,
            maximum_score: 15,
            point_budget: 30,
            attributes,
            action_rules: actions,
        }
    }

    #[tokio::test]
    async fn game_rule_http_calls_pin_requested_prompt_version_in_query() {
        use axum::{
            extract::Query,
            routing::{get, post},
            Json, Router,
        };

        let seen = Arc::new(Mutex::new(Vec::new()));
        let post_seen = seen.clone();
        let get_seen = seen.clone();
        let template = game_rule_template(BASIC_GAME_RULE_PROMPT_VERSION);
        let app = Router::new()
            .route(
                "/internal/novels/{id}/game-rules",
                post(move |Query(query): Query<HashMap<String, String>>| {
                    let seen = post_seen.clone();
                    let template = template.clone();
                    async move {
                        seen.lock()
                            .unwrap()
                            .push(query.get("prompt_version").cloned());
                        Json(template)
                    }
                }),
            )
            .route(
                "/internal/novels/{id}/game-rules/{version}",
                get(move |Query(query): Query<HashMap<String, String>>| {
                    let seen = get_seen.clone();
                    let template = game_rule_template(BASIC_GAME_RULE_PROMPT_VERSION);
                    async move {
                        seen.lock()
                            .unwrap()
                            .push(query.get("prompt_version").cloned());
                        Json(template)
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = NovelServiceClient::new(format!("http://{address}"), "test-token".into());

        client
            .request_game_rule_template(NOVEL_ID, USER_ID, BASIC_GAME_RULE_PROMPT_VERSION)
            .await
            .unwrap();
        client
            .get_game_rule_template(
                NOVEL_ID,
                1,
                USER_ID,
                BASIC_GAME_RULE_PROMPT_VERSION,
                None,
                false,
            )
            .await
            .unwrap()
            .unwrap();

        assert_eq!(
            *seen.lock().unwrap(),
            vec![
                Some(BASIC_GAME_RULE_PROMPT_VERSION.into()),
                Some(BASIC_GAME_RULE_PROMPT_VERSION.into()),
            ]
        );
        server.abort();
    }

    #[tokio::test]
    async fn game_rule_http_calls_reject_a_different_returned_prompt_version() {
        use axum::{
            routing::{get, post},
            Json, Router,
        };

        let template = game_rule_template(GAME_RULE_PROMPT_VERSION);
        let app = Router::new()
            .route(
                "/internal/novels/{id}/game-rules",
                post(move || async move { Json(template.clone()) }),
            )
            .route(
                "/internal/novels/{id}/game-rules/{version}",
                get(move || async move { Json(game_rule_template(GAME_RULE_PROMPT_VERSION)) }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = NovelServiceClient::new(format!("http://{address}"), "test-token".into());

        assert!(client
            .request_game_rule_template(NOVEL_ID, USER_ID, BASIC_GAME_RULE_PROMPT_VERSION)
            .await
            .is_err());
        assert!(client
            .get_game_rule_template(
                NOVEL_ID,
                1,
                USER_ID,
                BASIC_GAME_RULE_PROMPT_VERSION,
                None,
                false
            )
            .await
            .is_err());
        server.abort();
    }

    #[tokio::test]
    async fn series_http_rules_keep_source_identity_and_require_exact_authorized_binding() {
        use crate::domain::entities::game_rules::SeriesRuleContext;
        use axum::{
            extract::{Path, Query},
            routing::{get, post},
            Json, Router,
        };
        use std::{
            collections::HashMap,
            sync::{Arc, Mutex},
        };
        let binding = SeriesRuleBinding {
            series_id: Uuid::new_v4(),
            revision: 1,
        };
        let mut template = game_rule_template(BASIC_GAME_RULE_PROMPT_VERSION);
        template.novel_id = Uuid::new_v4();
        template.canon_model_version = 7;
        template.prompt_version = SERIES_GAME_RULE_PROMPT_VERSION.into();
        template.series = Some(SeriesRuleContext {
            binding: binding.clone(),
            target_novel_id: NOVEL_ID,
            name: "暮城系列".into(),
            background: "城门附近的共同设定。".into(),
        });
        let response = Arc::new(Mutex::new(template.clone()));
        let seen = Arc::new(Mutex::new(Vec::<HashMap<String, String>>::new()));
        let post_seen = seen.clone();
        let post_response = response.clone();
        let get_seen = seen.clone();
        let get_response = response.clone();
        let app = Router::new()
            .route(
                "/internal/novels/{id}/game-rules",
                post(
                    move |Path(id): Path<Uuid>, Query(query): Query<HashMap<String, String>>| {
                        assert_eq!(id, NOVEL_ID);
                        post_seen.lock().unwrap().push(query);
                        let template = post_response.lock().unwrap().clone();
                        async move { Json(template) }
                    },
                ),
            )
            .route(
                "/internal/novels/{id}/game-rules/{version}",
                get(
                    move |Path((id, version)): Path<(Uuid, i32)>,
                          Query(query): Query<HashMap<String, String>>| {
                        assert_eq!(id, NOVEL_ID);
                        assert_eq!(version, 7);
                        get_seen.lock().unwrap().push(query);
                        let template = get_response.lock().unwrap().clone();
                        async move { Json(template) }
                    },
                ),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = NovelServiceClient::new(format!("http://{address}"), "test-token".into());
        assert_eq!(
            client
                .request_game_rule_template(NOVEL_ID, USER_ID, BASIC_GAME_RULE_PROMPT_VERSION)
                .await
                .unwrap(),
            template
        );
        for current in [true, false] {
            assert_eq!(
                client
                    .get_game_rule_template(
                        NOVEL_ID,
                        7,
                        USER_ID,
                        SERIES_GAME_RULE_PROMPT_VERSION,
                        Some(&binding),
                        current
                    )
                    .await
                    .unwrap()
                    .unwrap(),
                template
            );
        }
        {
            let queries = seen.lock().unwrap();
            assert_eq!(
                queries[0].get("prefer_series").map(String::as_str),
                Some("true")
            );
            assert_eq!(
                queries[1].get("series_id"),
                Some(&binding.series_id.to_string())
            );
            assert_eq!(
                queries[1].get("series_revision").map(String::as_str),
                Some("1")
            );
            assert_eq!(
                queries[1].get("require_current_series").map(String::as_str),
                Some("true")
            );
            assert_eq!(
                queries[2].get("require_current_series").map(String::as_str),
                Some("false")
            );
        }
        for invalid in 0..3 {
            let mut forged = template.clone();
            match invalid {
                0 => forged.series.as_mut().unwrap().target_novel_id = Uuid::new_v4(),
                1 => forged.series.as_mut().unwrap().binding.series_id = Uuid::new_v4(),
                _ => forged.canon_model_version = 2,
            }
            *response.lock().unwrap() = forged;
            assert!(client
                .get_game_rule_template(
                    NOVEL_ID,
                    7,
                    USER_ID,
                    SERIES_GAME_RULE_PROMPT_VERSION,
                    Some(&binding),
                    false
                )
                .await
                .is_err());
        }
        let requests = seen.lock().unwrap().len();
        assert!(client
            .get_game_rule_template(
                NOVEL_ID,
                7,
                USER_ID,
                SERIES_GAME_RULE_PROMPT_VERSION,
                None,
                false
            )
            .await
            .is_err());
        assert!(client
            .get_game_rule_template(
                NOVEL_ID,
                7,
                USER_ID,
                BASIC_GAME_RULE_PROMPT_VERSION,
                Some(&binding),
                false
            )
            .await
            .is_err());
        let invalid = SeriesRuleBinding {
            series_id: binding.series_id,
            revision: 2,
        };
        assert!(client
            .get_game_rule_template(
                NOVEL_ID,
                7,
                USER_ID,
                SERIES_GAME_RULE_PROMPT_VERSION,
                Some(&invalid),
                false
            )
            .await
            .is_err());
        assert!(client
            .request_game_rule_template(NOVEL_ID, USER_ID, SERIES_GAME_RULE_PROMPT_VERSION)
            .await
            .is_err());
        assert_eq!(seen.lock().unwrap().len(), requests);
        server.abort();
    }

    #[tokio::test]
    async fn internal_rule_calls_do_not_forward_credentials_through_redirects() {
        use axum::{
            http::{header::LOCATION, StatusCode},
            routing::get,
            Router,
        };
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };
        let forwarded = Arc::new(AtomicUsize::new(0));
        let counted = forwarded.clone();
        let app = Router::new()
            .route(
                "/internal/novels/{id}/game-rules/{version}",
                get(|| async { (StatusCode::FOUND, [(LOCATION, "/redirect-target")]) }),
            )
            .route(
                "/redirect-target",
                get(move || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    async { StatusCode::SERVICE_UNAVAILABLE }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let client = NovelServiceClient::new(format!("http://{address}"), "test-token".into());
        assert!(client
            .get_game_rule_template(
                NOVEL_ID,
                1,
                USER_ID,
                BASIC_GAME_RULE_PROMPT_VERSION,
                None,
                false
            )
            .await
            .is_err());
        assert_eq!(forwarded.load(Ordering::SeqCst), 0);
        server.abort();
    }

    #[tokio::test]
    async fn game_rules_progress_rejection_is_distinct_from_service_failure() {
        use axum::{http::StatusCode, routing::post, Json, Router};

        for (status, code, hidden, sources_unavailable, canon_unavailable) in [
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "game_rules_unavailable_at_progress",
                true,
                false,
                false,
            ),
            (
                StatusCode::BAD_GATEWAY,
                "game_rules_unavailable_at_progress",
                false,
                false,
                false,
            ),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "game_rule_sources_unavailable",
                false,
                true,
                false,
            ),
            (
                StatusCode::CONFLICT,
                "canon_unavailable",
                false,
                false,
                true,
            ),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "unknown_error",
                false,
                false,
                false,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let app = Router::new().route(
                "/internal/novels/{id}/game-rules",
                post(move || async move {
                    (
                        status,
                        Json(serde_json::json!({"error": {
                            "code": code, "message": "private upstream detail"
                        }})),
                    )
                }),
            );
            let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
            let client = NovelServiceClient::new(format!("http://{address}"), "test-token".into());
            let error = client
                .request_game_rule_template(
                    NOVEL_ID,
                    USER_ID,
                    crate::domain::entities::game_rules::BASIC_GAME_RULE_PROMPT_VERSION,
                )
                .await
                .unwrap_err();
            assert_eq!(
                matches!(error, GameRuleTemplateRequestError::UnavailableAtProgress),
                hidden
            );
            assert_eq!(
                matches!(error, GameRuleTemplateRequestError::SourcesUnavailable),
                sources_unavailable
            );
            assert_eq!(
                matches!(error, GameRuleTemplateRequestError::CanonUnavailable),
                canon_unavailable
            );
            assert!(!error.to_string().contains("private upstream detail"));
            server.abort();
        }
    }

    fn progress() -> ReadingProgressResponse {
        ReadingProgressResponse {
            user_id: USER_ID,
            novel_id: NOVEL_ID,
            current_chapter: 3,
            reader_identity_type: "self".into(),
        }
    }

    #[test]
    fn validates_reading_progress_for_requested_scope() {
        let snapshot = validate_reading_progress(progress(), NOVEL_ID, USER_ID).unwrap();

        assert_eq!(
            snapshot,
            ReadingProgressSnapshot {
                current_chapter: 3,
                reader_identity_is_self: true,
            }
        );
    }

    #[test]
    fn rejects_reading_progress_for_another_user() {
        let error =
            validate_reading_progress(progress(), NOVEL_ID, Uuid::from_u128(3)).unwrap_err();

        assert!(error.to_string().contains("another user"));
    }

    #[test]
    fn rejects_reading_progress_for_another_novel() {
        let error = validate_reading_progress(progress(), Uuid::from_u128(3), USER_ID).unwrap_err();

        assert!(error.to_string().contains("another novel"));
    }

    #[test]
    fn rejects_invalid_reading_progress_chapter() {
        let mut progress = progress();
        progress.current_chapter = 0;

        let error = validate_reading_progress(progress, NOVEL_ID, USER_ID).unwrap_err();

        assert!(error.to_string().contains("invalid current chapter"));
    }

    #[test]
    fn rejects_invalid_reader_identity_type() {
        let mut progress = progress();
        progress.reader_identity_type = "unknown".into();

        let error = validate_reading_progress(progress, NOVEL_ID, USER_ID).unwrap_err();

        assert!(error.to_string().contains("invalid reader identity type"));
    }
}
