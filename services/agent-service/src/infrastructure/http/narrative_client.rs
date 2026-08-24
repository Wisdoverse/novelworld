use std::{collections::HashSet, time::Duration};

use anyhow::{anyhow, ensure, Result};
use async_trait::async_trait;
use reqwest::Client;
use uuid::Uuid;

use crate::domain::ports::{CharacterWorldContext, ReadinessProbe, WorldContextPort};

const NARRATIVE_SERVICE_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_CONTEXT_BYTES: usize = 64 * 1024;
const MAX_CONTEXT_CHARS: usize = 8_000;
const MAX_RECENT_ACTIONS: usize = 4;
const WORLD_CONTEXT_VERSION_HEADER: &str = "X-World-Context-Version";
const WORLD_CONTEXT_VERSION: &str = "2";

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

    fn world_context_request(&self, url: &str, user_id: Uuid) -> reqwest::RequestBuilder {
        self.client
            .get(url)
            .header("X-User-Id", user_id.to_string())
            .header("X-Internal-Service-Token", &self.internal_service_token)
            .header(WORLD_CONTEXT_VERSION_HEADER, WORLD_CONTEXT_VERSION)
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
            .world_context_request(&url, user_id)
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
        decode_context(&body, user_id, novel_id, character_id)
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

fn decode_context(
    body: &[u8],
    user_id: Uuid,
    novel_id: Uuid,
    character_id: Uuid,
) -> Result<Option<CharacterWorldContext>> {
    ensure!(
        body.len() <= MAX_CONTEXT_BYTES,
        "world context is oversized"
    );
    let context = serde_json::from_slice::<CharacterWorldContext>(body)?;
    validate_context_scope(&context, user_id, novel_id, character_id)?;
    if context.source_chapter_high_water.is_none() {
        tracing::warn!("omitting legacy world context without a canonical source high-water mark");
        return Ok(None);
    }
    validate_context(&context, user_id, novel_id, character_id)?;
    Ok(Some(context))
}

fn validate_context_scope(
    context: &CharacterWorldContext,
    user_id: Uuid,
    novel_id: Uuid,
    character_id: Uuid,
) -> Result<()> {
    ensure!(context.user_id == user_id);
    ensure!(context.novel_id == novel_id);
    ensure!(context.character_id == character_id);
    Ok(())
}

fn validate_context(
    context: &CharacterWorldContext,
    user_id: Uuid,
    novel_id: Uuid,
    character_id: Uuid,
) -> Result<()> {
    validate_context_scope(context, user_id, novel_id, character_id)?;
    ensure!(context.canon_model_version >= 1 && context.checkpoint_chapter >= 1);
    let source_chapter_high_water = context
        .source_chapter_high_water
        .ok_or_else(|| anyhow!("world context source high-water is missing"))?;
    ensure!(source_chapter_high_water >= context.checkpoint_chapter);
    ensure!(context.turn_number >= 0 && context.world_time >= 0);
    ensure!(!context.player_id.is_nil());
    ensure!(serde_json::to_string(context)?.chars().count() <= MAX_CONTEXT_CHARS);
    bounded_token(&context.player_name, 100)?;
    bounded_token(&context.player_location_id, 200)?;
    // Preserve the pre-choice wire bounds during rolling deploys. The new
    // narrative producer emits a smaller context; the existing Agent prompt
    // budget remains the final aggregate guard.
    ensure!(context.goals.len() <= 256);
    ensure!(context.recent_actions.len() <= MAX_RECENT_ACTIONS);
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
        validate_chapters(&goal.source_chapters, source_chapter_high_water)?;
    }
    if let Some(event) = &context.current_canonical_event {
        bounded_token(&event.id, 200)?;
        bounded_text(&event.summary, 1_000)?;
        ensure!(event.character_ids.contains(&character_id));
        ensure!(matches!(event.status.as_str(), "scheduled" | "delayed"));
        ensure!(event.character_ids.len() <= 256 && event.death_character_ids.len() <= 256);
        ensure!(event.location_ids.len() <= 256 && event.faction_ids.len() <= 256);
        validate_chapters(&event.source_chapters, source_chapter_high_water)?;
        if let Some(reason) = &event.reason {
            bounded_text(reason, 1_000)?;
        }
    }
    for action in &context.recent_actions {
        ensure!(!action.turn_id.is_nil());
        ensure!((1..=context.turn_number).contains(&action.turn_number));
        ensure!(matches!(
            action.action.kind.as_str(),
            "converse" | "ally" | "oppose"
        ));
        let target_id = action
            .action
            .target_id
            .as_deref()
            .ok_or_else(|| anyhow!("character-directed action target is missing"))?;
        bounded_token(target_id, 200)?;
        ensure!(Uuid::parse_str(target_id)? == character_id);
    }
    ensure!(context
        .recent_actions
        .windows(2)
        .all(|actions| actions[0].turn_number < actions[1].turn_number));
    let mut action_ids = HashSet::new();
    ensure!(context
        .recent_actions
        .iter()
        .all(|action| action_ids.insert(action.turn_id)));
    for event in &context.recent_player_events {
        bounded_token(&event.id, 300)?;
        bounded_text(&event.summary, 1_000)?;
        ensure!(event.turn_number >= 1 && event.turn_number <= context.turn_number);
        ensure!(event.world_time >= 1 && event.world_time <= context.world_time);
        ensure!(event.actor_character_ids.len() <= 16);
        ensure!(event.actor_character_ids.contains(&character_id));
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
    use crate::domain::ports::{
        WorldActionContext, WorldActionData, WorldActiveThread, WorldCanonicalEvent,
        WorldCharacterGoal, WorldHistoryItem,
    };

    #[test]
    fn world_context_request_declares_the_fail_closed_context_version() {
        let client = NarrativeServiceClient::new("http://narrative.invalid".into(), "token".into());
        let request = client
            .world_context_request("http://narrative.invalid/context", Uuid::new_v4())
            .build()
            .unwrap();

        assert_eq!(
            request
                .headers()
                .get(WORLD_CONTEXT_VERSION_HEADER)
                .and_then(|value| value.to_str().ok()),
            Some(WORLD_CONTEXT_VERSION)
        );
    }

    fn valid_context(user_id: Uuid, novel_id: Uuid, character_id: Uuid) -> CharacterWorldContext {
        CharacterWorldContext {
            user_id,
            novel_id,
            character_id,
            character_alive: true,
            canon_model_version: 1,
            checkpoint_chapter: 2,
            source_chapter_high_water: Some(2),
            turn_number: 1,
            world_time: 1,
            player_id: Uuid::new_v4(),
            player_name: "云舟".into(),
            player_location_id: "gate".into(),
            relationship: None,
            goals: vec![],
            perception_of_player: None,
            current_canonical_event: None,
            recent_actions: vec![],
            recent_player_events: vec![],
            active_threads: vec![],
        }
    }

    #[test]
    fn world_context_scope_and_bounds_fail_closed() {
        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let mut context = valid_context(user_id, novel_id, character_id);
        validate_context(&context, user_id, novel_id, character_id).unwrap();
        context.character_alive = false;
        validate_context(&context, user_id, novel_id, character_id).unwrap();
        context.character_id = Uuid::new_v4();
        assert!(validate_context(&context, user_id, novel_id, character_id).is_err());
    }

    #[test]
    fn recent_events_must_explicitly_include_the_requested_character() {
        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let mut context = valid_context(user_id, novel_id, character_id);
        context.recent_player_events.push(WorldHistoryItem {
            id: "private-event".into(),
            turn_id: Uuid::new_v4(),
            turn_number: 1,
            world_time: 1,
            summary: "另一角色在异地秘密行动".into(),
            actor_character_ids: vec![Uuid::new_v4()],
            location_id: Some("elsewhere".into()),
        });
        assert!(validate_context(&context, user_id, novel_id, character_id).is_err());

        context.recent_player_events[0].actor_character_ids = vec![character_id];
        validate_context(&context, user_id, novel_id, character_id).unwrap();
    }

    #[test]
    fn missing_source_high_water_remains_deserializable_but_unproven() {
        let context = valid_context(Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut legacy = serde_json::to_value(context).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("source_chapter_high_water");
        let legacy: CharacterWorldContext = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.source_chapter_high_water, None);
    }

    #[test]
    fn legacy_context_with_unproven_future_fields_is_omitted() {
        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let mut context = valid_context(user_id, novel_id, character_id);
        context.checkpoint_chapter = 1;
        context.source_chapter_high_water = None;
        context.current_canonical_event = Some(WorldCanonicalEvent {
            id: "future-event".into(),
            sequence: 1,
            summary: "来自第二章的事件".into(),
            character_ids: vec![character_id],
            location_ids: vec![],
            faction_ids: vec![],
            death_character_ids: vec![],
            source_chapters: vec![2],
            status: "scheduled".into(),
            reason: None,
        });

        let body = serde_json::to_vec(&context).unwrap();
        assert!(decode_context(&body, user_id, novel_id, character_id)
            .unwrap()
            .is_none());
    }

    #[test]
    fn recent_actions_are_committed_bounded_and_default_for_a_rolling_deploy() {
        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let mut context = valid_context(user_id, novel_id, character_id);
        let action = WorldActionContext {
            turn_id: Uuid::new_v4(),
            turn_number: 1,
            action: WorldActionData {
                kind: "converse".into(),
                target_id: Some(character_id.to_string()),
            },
        };
        context.recent_actions.push(action.clone());
        validate_context(&context, user_id, novel_id, character_id).unwrap();

        let private_intent = "PRIVATE-INTENT-PRETEND-TO-ALLY-THEN-BETRAY";
        let mut older_producer = serde_json::to_value(&context).unwrap();
        older_producer["recent_actions"][0]["action"]["intent"] = private_intent.into();
        let older_producer: CharacterWorldContext = serde_json::from_value(older_producer).unwrap();
        validate_context(&older_producer, user_id, novel_id, character_id).unwrap();
        assert!(!serde_json::to_string(&older_producer)
            .unwrap()
            .contains(private_intent));

        let mut legacy = serde_json::to_value(&context).unwrap();
        legacy.as_object_mut().unwrap().remove("recent_actions");
        let legacy: CharacterWorldContext = serde_json::from_value(legacy).unwrap();
        assert!(legacy.recent_actions.is_empty());

        context.recent_actions = vec![action; MAX_RECENT_ACTIONS + 1];
        assert!(validate_context(&context, user_id, novel_id, character_id).is_err());
        context.recent_actions.truncate(1);
        let duplicate = context.recent_actions[0].clone();
        context.recent_actions.push(duplicate.clone());
        assert!(validate_context(&context, user_id, novel_id, character_id).is_err());
        context.recent_actions[1].turn_number = 2;
        context.turn_number = 2;
        assert!(validate_context(&context, user_id, novel_id, character_id).is_err());
        context.recent_actions[1].turn_id = Uuid::new_v4();
        validate_context(&context, user_id, novel_id, character_id).unwrap();
    }

    #[test]
    fn recent_actions_must_be_character_directed_to_the_requested_character() {
        let user_id = Uuid::from_u128(1);
        let novel_id = Uuid::from_u128(2);
        let character_id = Uuid::from_u128(3);
        let mut context = valid_context(user_id, novel_id, character_id);
        context.recent_actions.push(WorldActionContext {
            turn_id: Uuid::from_u128(4),
            turn_number: 1,
            action: WorldActionData {
                kind: "converse".into(),
                target_id: Some(character_id.to_string()),
            },
        });

        for kind in ["converse", "ally", "oppose"] {
            context.recent_actions[0].action.kind = kind.into();
            validate_context(&context, user_id, novel_id, character_id).unwrap();
        }

        context.recent_actions[0].action.kind = "investigate".into();
        assert!(validate_context(&context, user_id, novel_id, character_id).is_err());
        context.recent_actions[0].action.kind = "converse".into();

        for target_id in [
            None,
            Some("not-a-uuid".into()),
            Some(Uuid::from_u128(5).to_string()),
        ] {
            context.recent_actions[0].action.target_id = target_id;
            assert!(validate_context(&context, user_id, novel_id, character_id).is_err());
        }
    }

    #[test]
    fn aggregate_world_context_must_leave_room_in_the_agent_prompt() {
        const TEST_ITEMS: usize = 4;
        const LEGACY_TEXT_CHARS: usize = 1_000;
        let user_id = Uuid::new_v4();
        let novel_id = Uuid::new_v4();
        let character_id = Uuid::new_v4();
        let mut context = valid_context(user_id, novel_id, character_id);
        context.goals = (0..TEST_ITEMS)
            .map(|index| WorldCharacterGoal {
                id: format!("goal-{index}"),
                character_id,
                description: "目标".repeat(LEGACY_TEXT_CHARS / 2),
                source_chapters: vec![1],
            })
            .collect();
        context.turn_number = TEST_ITEMS as i64;
        context.world_time = TEST_ITEMS as i64;
        context.recent_player_events = (1..=TEST_ITEMS)
            .map(|turn| WorldHistoryItem {
                id: format!("event-{turn}"),
                turn_id: Uuid::new_v4(),
                turn_number: turn as i64,
                world_time: turn as i64,
                summary: "事件".repeat(LEGACY_TEXT_CHARS / 2),
                actor_character_ids: vec![character_id],
                location_id: Some("gate".into()),
            })
            .collect();
        context.active_threads = (0..TEST_ITEMS)
            .map(|index| WorldActiveThread {
                id: format!("thread-{index}"),
                description: "线索".repeat(LEGACY_TEXT_CHARS / 2),
                origin: "canon".into(),
            })
            .collect();
        assert!(serde_json::to_string(&context).unwrap().chars().count() > MAX_CONTEXT_CHARS);
        assert!(validate_context(&context, user_id, novel_id, character_id).is_err());
    }
}
