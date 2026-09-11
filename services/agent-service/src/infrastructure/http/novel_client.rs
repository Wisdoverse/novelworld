use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::{Bytes, BytesMut};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use uuid::Uuid;

use crate::domain::ports::{
    LoreContextPort, LoreExcerpt, ReadinessProbe, ReadingContext, ReadingContextPort,
};
use crate::domain::repositories::{
    CharacterCanonGrounding, CharacterInfo, CharacterInfoRepository,
};

const NOVEL_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CANON_GROUNDING_BYTES: usize = 32 * 1024;

/// HTTP adapter that fetches character info from novel-service,
/// replacing the previous direct-DB query against novel-service's tables.
pub struct NovelServiceClient {
    client: Client,
    base_url: String,
    internal_service_token: String,
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
    world_summary: Option<String>,
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
            world_summary: ch.world_summary,
            first_appearance_chapter: ch.first_appearance_chapter,
        }))
    }

    async fn find_canon_grounding(
        &self,
        novel_id: Uuid,
        character_id: Uuid,
        checkpoint_chapter: i32,
        user_id: Uuid,
    ) -> Result<Option<CharacterCanonGrounding>> {
        let url = format!(
            "{}/internal/novels/{}/characters/{}/grounding-v1/{}",
            self.base_url, novel_id, character_id, checkpoint_chapter
        );
        // One bounded read, deliberately not retried: the chat claim records a
        // context failure and the client may explicitly retry its idempotency key.
        let response = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .send()
            .await
            .map_err(|error| anyhow!("Failed to reach novel-service at {}: {}", url, error))?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(anyhow!(
                "novel-service returned {} for character canon grounding",
                response.status()
            ));
        }
        let body = read_capped_body(response, MAX_CANON_GROUNDING_BYTES).await?;
        let grounding = decode_canon_grounding(&body, novel_id, character_id, checkpoint_chapter)?;
        Ok(Some(grounding))
    }
}

async fn read_capped_body(mut response: reqwest::Response, limit: usize) -> Result<Bytes> {
    anyhow::ensure!(
        response
            .content_length()
            .is_none_or(|length| length <= limit as u64),
        "novel-service canon grounding response exceeds its byte limit"
    );
    let mut body = BytesMut::with_capacity(limit.min(8 * 1024));
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            chunk.len() <= limit.saturating_sub(body.len()),
            "novel-service canon grounding response exceeds its byte limit"
        );
        body.extend_from_slice(&chunk);
    }
    Ok(body.freeze())
}

fn decode_canon_grounding(
    body: &[u8],
    novel_id: Uuid,
    character_id: Uuid,
    checkpoint_chapter: i32,
) -> Result<CharacterCanonGrounding> {
    anyhow::ensure!(
        body.len() <= MAX_CANON_GROUNDING_BYTES,
        "novel-service canon grounding response exceeds its byte limit"
    );
    let grounding = serde_json::from_slice::<CharacterCanonGrounding>(body)?;
    validate_canon_grounding(&grounding, novel_id, character_id, checkpoint_chapter)?;
    Ok(grounding)
}

fn validate_canon_grounding(
    grounding: &CharacterCanonGrounding,
    novel_id: Uuid,
    character_id: Uuid,
    checkpoint_chapter: i32,
) -> Result<()> {
    if grounding.novel_id != novel_id
        || grounding.model_version != 1
        || grounding.character_id != character_id
        || grounding.checkpoint_chapter != checkpoint_chapter
        || grounding.goals.len() > 6
        || grounding.relationships.len() > 8
    {
        return Err(anyhow!(
            "novel-service returned invalid canon grounding scope"
        ));
    }
    for goal in &grounding.goals {
        bounded_grounding_text(&goal.description, 200)?;
        validate_source_chapters(&goal.source_chapters, checkpoint_chapter)?;
    }
    for relationship in &grounding.relationships {
        if relationship.other_character_id.is_nil()
            || relationship.other_character_id == character_id
            || !matches!(relationship.direction.as_str(), "incoming" | "outgoing")
        {
            return Err(anyhow!("novel-service returned invalid canon relationship"));
        }
        bounded_grounding_text(&relationship.other_character_name, 100)?;
        bounded_grounding_text(&relationship.kind, 50)?;
        bounded_grounding_text(&relationship.description, 200)?;
        validate_source_chapters(&relationship.source_chapters, checkpoint_chapter)?;
    }
    Ok(())
}

fn bounded_grounding_text(value: &str, max_chars: usize) -> Result<()> {
    if value.trim().is_empty()
        || value.chars().count() > max_chars
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(anyhow!(
            "novel-service returned invalid canon grounding text"
        ));
    }
    Ok(())
}

fn validate_source_chapters(chapters: &[i32], checkpoint_chapter: i32) -> Result<()> {
    if chapters.is_empty()
        || chapters.len() > 8
        || chapters
            .iter()
            .any(|chapter| !(1..=checkpoint_chapter).contains(chapter))
        || chapters.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(anyhow!(
            "novel-service returned invalid canon grounding provenance"
        ));
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::repositories::{CharacterCanonGoal, CharacterCanonRelationship};
    use axum::{body::Body, routing::get, Router};
    use std::convert::Infallible;

    fn grounding() -> CharacterCanonGrounding {
        CharacterCanonGrounding {
            novel_id: Uuid::new_v4(),
            model_version: 1,
            checkpoint_chapter: 3,
            character_id: Uuid::new_v4(),
            goals: vec![CharacterCanonGoal {
                description: "守住城门".into(),
                source_chapters: vec![1, 3],
            }],
            relationships: vec![CharacterCanonRelationship {
                other_character_id: Uuid::new_v4(),
                other_character_name: "顾衡".into(),
                direction: "outgoing".into(),
                kind: "盟友".into(),
                description: "共同守城".into(),
                source_chapters: vec![2],
            }],
        }
    }

    #[test]
    fn canon_grounding_validation_is_exact_and_source_bounded() {
        let valid = grounding();
        assert!(validate_canon_grounding(
            &valid,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_ok());

        let mut wrong_model = valid.clone();
        wrong_model.model_version = 2;
        assert!(validate_canon_grounding(
            &wrong_model,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        let mut wrong_novel = valid.clone();
        wrong_novel.novel_id = Uuid::new_v4();
        assert!(validate_canon_grounding(
            &wrong_novel,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        let mut wrong_character = valid.clone();
        wrong_character.character_id = Uuid::new_v4();
        assert!(validate_canon_grounding(
            &wrong_character,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        let mut wrong_checkpoint = valid.clone();
        wrong_checkpoint.checkpoint_chapter += 1;
        assert!(validate_canon_grounding(
            &wrong_checkpoint,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        let mut too_many_goals = valid.clone();
        too_many_goals.goals = vec![valid.goals[0].clone(); 7];
        assert!(validate_canon_grounding(
            &too_many_goals,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        let mut too_many_relationships = valid.clone();
        too_many_relationships.relationships = vec![valid.relationships[0].clone(); 9];
        assert!(validate_canon_grounding(
            &too_many_relationships,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        let mut future = valid.clone();
        future.goals[0].source_chapters = vec![1, 4];
        assert!(validate_canon_grounding(
            &future,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        let mut duplicate = valid.clone();
        duplicate.relationships[0].source_chapters = vec![2, 2];
        assert!(validate_canon_grounding(
            &duplicate,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        for invalid_description in ["   ".into(), "x".repeat(201), "bad\0text".into()] {
            let mut invalid_text = valid.clone();
            invalid_text.goals[0].description = invalid_description;
            assert!(validate_canon_grounding(
                &invalid_text,
                valid.novel_id,
                valid.character_id,
                valid.checkpoint_chapter
            )
            .is_err());
        }

        let mut invalid_relationship = valid.clone();
        invalid_relationship.relationships[0].other_character_name = "x".repeat(101);
        assert!(validate_canon_grounding(
            &invalid_relationship,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());
        invalid_relationship = valid.clone();
        invalid_relationship.relationships[0].kind = "bad\0kind".into();
        assert!(validate_canon_grounding(
            &invalid_relationship,
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        for source_chapters in [Vec::new(), vec![2, 1]] {
            let mut invalid_sources = valid.clone();
            invalid_sources.goals[0].source_chapters = source_chapters;
            assert!(validate_canon_grounding(
                &invalid_sources,
                valid.novel_id,
                valid.character_id,
                valid.checkpoint_chapter
            )
            .is_err());
        }

        let mut unknown_field = serde_json::to_value(&valid).unwrap();
        unknown_field["unexpected"] = serde_json::json!(true);
        assert!(decode_canon_grounding(
            &serde_json::to_vec(&unknown_field).unwrap(),
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());

        assert!(decode_canon_grounding(
            &vec![b'x'; MAX_CANON_GROUNDING_BYTES + 1],
            valid.novel_id,
            valid.character_id,
            valid.checkpoint_chapter
        )
        .is_err());
    }

    #[tokio::test]
    async fn canon_grounding_transport_stops_at_the_byte_limit() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().fallback(get(|| async {
            let body = Body::from_stream(async_stream::stream! {
                yield Ok::<_, Infallible>(Bytes::from(vec![b'x'; MAX_CANON_GROUNDING_BYTES + 1]));
                std::future::pending::<()>().await;
            });
            axum::response::Response::new(body)
        }));
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let valid = grounding();
        let client = NovelServiceClient::new(format!("http://{address}"), "token".into());

        let error = tokio::time::timeout(
            Duration::from_secs(1),
            client.find_canon_grounding(
                valid.novel_id,
                valid.character_id,
                valid.checkpoint_chapter,
                Uuid::new_v4(),
            ),
        )
        .await
        .expect("reader waited for bytes beyond the cap")
        .unwrap_err();
        server.abort();

        assert!(error.to_string().contains("exceeds its byte limit"));
    }
}
