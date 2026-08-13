use std::time::Duration;

use anyhow::{anyhow, ensure, Result};
use async_trait::async_trait;
use reqwest::Client;
use uuid::Uuid;

use crate::domain::ports::{CharacterWorldContext, ReadinessProbe, WorldContextPort};

const NARRATIVE_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTEXT_BYTES: usize = 64 * 1024;

pub struct NarrativeServiceClient {
    client: Client,
    base_url: String,
    internal_service_token: String,
}

impl NarrativeServiceClient {
    pub fn new(base_url: String, internal_service_token: String) -> Self {
        Self {
            client: Client::builder()
                .timeout(NARRATIVE_SERVICE_TIMEOUT)
                .build()
                .expect("valid narrative-service HTTP client configuration"),
            base_url,
            internal_service_token,
        }
    }
}

#[async_trait]
impl WorldContextPort for NarrativeServiceClient {
    async fn find(
        &self,
        novel_id: Uuid,
        character_id: Uuid,
        user_id: Uuid,
    ) -> Result<Option<CharacterWorldContext>> {
        let url = format!(
            "{}/internal/narrative/{}/characters/{}/context",
            self.base_url, novel_id, character_id
        );
        let response = self
            .client
            .get(&url)
            .header("X-User-Id", user_id.to_string())
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .send()
            .await
            .map_err(|error| anyhow!("narrative-service world context unavailable: {error}"))?;
        if response.status() == reqwest::StatusCode::NO_CONTENT
            || response.status() == reqwest::StatusCode::NOT_FOUND
        {
            return Ok(None);
        }
        if !response.status().is_success() {
            return Err(anyhow!(
                "narrative-service returned {} for world context",
                response.status()
            ));
        }
        let body = response.bytes().await?;
        ensure!(
            body.len() <= MAX_CONTEXT_BYTES,
            "world context is oversized"
        );
        let context = serde_json::from_slice::<CharacterWorldContext>(&body)?;
        validate_context(&context, user_id, novel_id, character_id)?;
        Ok(Some(context))
    }
}

#[async_trait]
impl ReadinessProbe for NarrativeServiceClient {
    async fn is_ready(&self) -> bool {
        self.client
            .get(format!("{}/ready", self.base_url))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
    }
}

fn validate_context(
    context: &CharacterWorldContext,
    user_id: Uuid,
    novel_id: Uuid,
    character_id: Uuid,
) -> Result<()> {
    ensure!(context.user_id == user_id);
    ensure!(context.novel_id == novel_id);
    ensure!(context.character_id == character_id);
    ensure!(context.canon_model_version >= 1 && context.checkpoint_chapter >= 1);
    ensure!(context.turn_number >= 0 && context.world_time >= 0);
    ensure!(!context.player_id.is_nil());
    bounded_token(&context.player_name, 100)?;
    bounded_token(&context.player_location_id, 200)?;
    ensure!(context.goals.len() <= 256);
    ensure!(context.recent_player_events.len() <= 16);
    ensure!(context.active_threads.len() <= 32);
    if let Some(relationship) = &context.relationship {
        ensure!((0..=100).contains(&relationship.score));
        bounded_text(&relationship.last_change, 1_000)?;
    }
    if let Some(perception) = &context.perception_of_player {
        bounded_text(perception, 1_000)?;
    }
    for goal in &context.goals {
        ensure!(goal.character_id == character_id);
        bounded_token(&goal.id, 200)?;
        bounded_text(&goal.description, 1_000)?;
        validate_chapters(&goal.source_chapters, context.checkpoint_chapter)?;
    }
    if let Some(event) = &context.current_canonical_event {
        bounded_token(&event.id, 200)?;
        bounded_text(&event.summary, 1_000)?;
        ensure!(event.character_ids.contains(&character_id));
        ensure!(matches!(event.status.as_str(), "scheduled" | "delayed"));
        ensure!(event.character_ids.len() <= 256 && event.death_character_ids.len() <= 256);
        ensure!(event.location_ids.len() <= 256 && event.faction_ids.len() <= 256);
        ensure!(!event.source_chapters.is_empty() && event.source_chapters.len() <= 256);
        if let Some(reason) = &event.reason {
            bounded_text(reason, 1_000)?;
        }
    }
    for event in &context.recent_player_events {
        bounded_token(&event.id, 300)?;
        bounded_text(&event.summary, 1_000)?;
        ensure!(event.turn_number >= 1 && event.turn_number <= context.turn_number);
        ensure!(event.world_time >= 1 && event.world_time <= context.world_time);
        ensure!(event.actor_character_ids.len() <= 16);
        if let Some(location) = &event.location_id {
            bounded_token(location, 200)?;
        }
    }
    for thread in &context.active_threads {
        bounded_token(&thread.id, 200)?;
        bounded_text(&thread.description, 1_000)?;
        ensure!(matches!(thread.origin.as_str(), "canon" | "player"));
    }
    Ok(())
}

fn validate_chapters(chapters: &[i32], maximum: i32) -> Result<()> {
    ensure!(!chapters.is_empty() && chapters.len() <= 256);
    ensure!(chapters.windows(2).all(|pair| pair[0] < pair[1]));
    ensure!(chapters
        .iter()
        .all(|chapter| (1..=maximum).contains(chapter)));
    Ok(())
}

fn bounded_token(value: &str, maximum: usize) -> Result<()> {
    bounded_text(value, maximum)?;
    ensure!(value.trim() == value && !value.chars().any(char::is_control));
    Ok(())
}

fn bounded_text(value: &str, maximum: usize) -> Result<()> {
    ensure!(!value.trim().is_empty() && value.chars().count() <= maximum);
    ensure!(value
        .chars()
        .all(|character| !character.is_control() || matches!(character, '\n' | '\r' | '\t')));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn world_context_scope_and_bounds_fail_closed() {
        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let mut context = CharacterWorldContext {
            user_id,
            novel_id,
            character_id,
            character_alive: true,
            canon_model_version: 1,
            checkpoint_chapter: 2,
            turn_number: 1,
            world_time: 1,
            player_id: Uuid::new_v4(),
            player_name: "云舟".into(),
            player_location_id: "gate".into(),
            relationship: None,
            goals: vec![],
            perception_of_player: None,
            current_canonical_event: None,
            recent_player_events: vec![],
            active_threads: vec![],
        };
        validate_context(&context, user_id, novel_id, character_id).unwrap();
        context.character_alive = false;
        validate_context(&context, user_id, novel_id, character_id).unwrap();
        context.character_id = Uuid::new_v4();
        assert!(validate_context(&context, user_id, novel_id, character_id).is_err());
    }
}
