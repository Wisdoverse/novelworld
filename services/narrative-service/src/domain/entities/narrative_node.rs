use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::domain::entities::game_rules::{ActionCheck, GameRuleTemplate};
use crate::domain::entities::player_entity::PlayerEntity;
use crate::domain::entities::world_session::{
    CanonicalEventStatus, CharacterWorldContext, WorldAction, WorldActionKind, WorldEntryContext,
    WorldHistoryItem, WorldSession, WorldTurnTransition, MAX_CHARACTER_CONTEXT_REFERENCES,
    MAX_CHARACTER_CONTEXT_SOURCE_CHAPTERS, MAX_CHARACTER_CONTEXT_TEXT_CHARS, MAX_CHARACTER_GOALS,
    MAX_CHARACTER_RECENT_EVENTS, MAX_CHARACTER_WORLD_CONTEXT_CHARS,
};
use crate::domain::services::narrative_transition::NarrativeTransition;

/// 叙事节点（关键分支点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeNode {
    pub id: Uuid,
    /// Runtime-generated nodes are player-scoped. `None` is retained only so
    /// already-committed legacy shared nodes remain exactly replayable.
    pub user_id: Option<Uuid>,
    pub novel_id: Uuid,
    pub chapter_number: i32,
    /// 节点描述（触发分支的情境）
    pub description: String,
    /// Exact source excerpt after which the choice is rendered inline.
    pub anchor_quote: Option<String>,
    /// 可选择的分支选项
    pub choices: Vec<NarrativeChoice>,
    pub created_at: DateTime<Utc>,
}

/// 分支选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeChoice {
    pub index: i32,
    pub text: String,
    /// 选择后的简短预告（不剧透）
    pub hint: String,
    /// 选择后 AI 生成的后续剧情（按需生成）
    pub generated_consequence: Option<String>,
}

impl NarrativeNode {
    pub fn new(
        novel_id: Uuid,
        chapter_number: i32,
        description: String,
        choices: Vec<NarrativeChoice>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            user_id: None,
            novel_id,
            chapter_number,
            description,
            anchor_quote: None,
            choices,
            created_at: Utc::now(),
        }
    }

    pub fn with_anchor_quote(mut self, anchor_quote: String) -> Self {
        self.anchor_quote = Some(anchor_quote);
        self
    }

    pub fn for_user(mut self, user_id: Uuid) -> Self {
        self.user_id = Some(user_id);
        self
    }
}

/// 世界状态（parallel-ai-engine 思路：持久化世界状态）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WorldState {
    pub user_id: Uuid,
    pub novel_id: Uuid,
    /// JSONB 存储：所有选择、关系变化、世界事件
    pub state: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, thiserror::Error)]
pub enum WorldStateError {
    #[error("timeline conflict: {0}")]
    TimelineConflict(String),
    #[error("world state choices must be an array")]
    InvalidChoices,
    #[error("world state {0} must be an object")]
    InvalidObject(&'static str),
    #[error("world state {0} must be an array")]
    InvalidArray(&'static str),
    #[error("invalid narrative transition: {0}")]
    InvalidTransition(String),
    #[error("invalid player entity: {0}")]
    InvalidPlayerEntity(String),
    #[error("invalid world session: {0}")]
    InvalidWorldSession(String),
}

impl WorldState {
    pub fn new(user_id: Uuid, novel_id: Uuid) -> Self {
        Self {
            user_id,
            novel_id,
            state: serde_json::json!({
                "choices": [],
                "relationships": {},
                "world_events": [],
                "reader_reputation": {}
            }),
            updated_at: Utc::now(),
        }
    }

    pub fn fingerprint(&self) -> [u8; 32] {
        Sha256::digest(self.state.to_string().as_bytes()).into()
    }

    pub fn latest_choice_chapter(&self) -> Result<Option<i32>, WorldStateError> {
        let choices = self
            .state
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .ok_or(WorldStateError::InvalidChoices)?;
        choices
            .iter()
            .map(|choice| {
                choice
                    .get("chapter")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|chapter| i32::try_from(chapter).ok())
                    .filter(|chapter| *chapter >= 1)
                    .ok_or(WorldStateError::InvalidChoices)
            })
            .collect::<Result<Vec<_>, _>>()
            .map(|chapters| chapters.into_iter().max())
    }

    /// Highest source chapter represented anywhere in the user-visible world
    /// state. Consumers compare this with current reading progress and omit the
    /// whole derived state after a rewind rather than partially leaking it.
    pub fn source_chapter_high_water(&self) -> Result<Option<i32>, WorldStateError> {
        let mut high_water = self.latest_choice_chapter()?;
        if let Some(player) = self.player_entity()? {
            high_water = Some(
                high_water.map_or(player.canonical_checkpoint_chapter, |chapter| {
                    chapter.max(player.canonical_checkpoint_chapter)
                }),
            );
        }
        if let Some(session) = self.open_world()? {
            high_water = Some(
                high_water.map_or(session.entry_context.unlocked_through_chapter, |chapter| {
                    chapter.max(session.entry_context.unlocked_through_chapter)
                }),
            );
        }
        Ok(high_water)
    }

    /// 记录读者的选择
    pub fn record_choice(
        &mut self,
        node_id: Uuid,
        chapter: i32,
        choice_index: i32,
        choice_text: &str,
        consequence: &str,
    ) -> Result<bool, WorldStateError> {
        self.player_entity()?;
        let choices = self
            .state
            .get_mut("choices")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or(WorldStateError::InvalidChoices)?;
        let node_id_string = node_id.to_string();

        if choices.iter().any(|choice| {
            choice.get("node_id").and_then(serde_json::Value::as_str)
                == Some(node_id_string.as_str())
        }) {
            return Ok(false);
        }

        if let Some(legacy) = choices.iter_mut().find(|choice| {
            choice.get("node_id").is_none()
                && choice.get("chapter").and_then(serde_json::Value::as_i64)
                    == Some(i64::from(chapter))
                && choice.get("choice").and_then(serde_json::Value::as_str) == Some(choice_text)
        }) {
            let object = legacy
                .as_object_mut()
                .ok_or(WorldStateError::InvalidChoices)?;
            object.insert("node_id".into(), node_id_string.into());
            object.insert("choice_index".into(), choice_index.into());
            object.insert("consequence".into(), consequence.into());
            self.updated_at = Utc::now();
            return Ok(true);
        }

        choices.push(serde_json::json!({
            "node_id": node_id,
            "chapter": chapter,
            "choice_index": choice_index,
            "choice": choice_text,
            "consequence": consequence,
            "timestamp": Utc::now().to_rfc3339(),
        }));
        self.updated_at = Utc::now();
        Ok(true)
    }

    pub fn apply_choice_transition(
        &mut self,
        node_id: Uuid,
        chapter: i32,
        choice_index: i32,
        choice_text: &str,
        transition: &NarrativeTransition,
    ) -> Result<bool, WorldStateError> {
        self.player_entity()?;
        transition
            .validate_shape()
            .map_err(|error| WorldStateError::InvalidTransition(error.to_string()))?;
        let mut next = self.state.clone();
        let node_id_string = node_id.to_string();
        {
            let choices = array_section(&mut next, "choices")?;
            if choices.iter().any(|choice| {
                choice.get("node_id").and_then(serde_json::Value::as_str)
                    == Some(node_id_string.as_str())
            }) {
                return Ok(false);
            }
            if let Some(legacy) = choices.iter_mut().find(|choice| {
                choice.get("node_id").is_none()
                    && choice.get("chapter").and_then(serde_json::Value::as_i64)
                        == Some(i64::from(chapter))
                    && choice.get("choice").and_then(serde_json::Value::as_str) == Some(choice_text)
            }) {
                let object = legacy
                    .as_object_mut()
                    .ok_or(WorldStateError::InvalidChoices)?;
                object.insert("node_id".into(), node_id_string.clone().into());
                object.insert("choice_index".into(), choice_index.into());
                object.insert(
                    "consequence".into(),
                    transition.rendered_narrative.clone().into(),
                );
                object.insert(
                    "canon_model_version".into(),
                    transition.canon_model_version.into(),
                );
                object.insert(
                    "canonical_checkpoint_chapter".into(),
                    transition.canonical_checkpoint_chapter.into(),
                );
            } else {
                choices.push(serde_json::json!({
                    "node_id": node_id,
                    "chapter": chapter,
                    "choice_index": choice_index,
                    "choice": choice_text,
                    "consequence": transition.rendered_narrative,
                    "canon_model_version": transition.canon_model_version,
                    "canonical_checkpoint_chapter": transition.canonical_checkpoint_chapter,
                }));
            }
        }
        {
            let relationships = relationship_section(&mut next)?;
            for change in &transition.relationship_changes {
                let key = change.character_id.to_string();
                let current = relationships
                    .get(&key)
                    .and_then(|value| value.get("score"))
                    .and_then(serde_json::Value::as_i64)
                    .unwrap_or(50) as i32;
                relationships.insert(
                    key,
                    serde_json::json!({
                        "score": (current + change.delta).clamp(0, 100),
                        "last_change": change.reason,
                    }),
                );
            }
        }
        {
            let events = array_section(&mut next, "world_events")?;
            for (index, event) in transition.events.iter().enumerate() {
                events.push(serde_json::json!({
                    "id": format!("{node_id}:event:{index}"),
                    "chapter": chapter,
                    "summary": event.summary,
                    "actor_character_ids": event.actor_character_ids,
                    "location_id": event.location_id,
                }));
            }
        }
        {
            let locations = object_section(&mut next, "locations")?;
            for change in &transition.location_changes {
                locations.insert(
                    change.location_id.clone(),
                    serde_json::json!({
                        "state": change.state,
                        "reason": change.reason,
                    }),
                );
            }
        }
        {
            let threads = object_section(&mut next, "threads")?;
            for change in &transition.thread_changes {
                threads.insert(
                    change.thread_id.clone(),
                    serde_json::json!({
                        "status": change.status.to_str(),
                        "description": change.description,
                    }),
                );
            }
        }
        self.state = next;
        self.updated_at = Utc::now();
        Ok(true)
    }

    pub fn player_entity(&self) -> Result<Option<PlayerEntity>, WorldStateError> {
        let root = self
            .state
            .as_object()
            .ok_or(WorldStateError::InvalidObject("root"))?;
        let Some(value) = root.get("player_entity") else {
            return Ok(None);
        };
        if value.is_null() {
            return Err(WorldStateError::InvalidPlayerEntity(
                "player entity must be an object".into(),
            ));
        }
        let entity = serde_json::from_value::<PlayerEntity>(value.clone())
            .map_err(|error| WorldStateError::InvalidPlayerEntity(error.to_string()))?;
        entity
            .validate()
            .map_err(|error| WorldStateError::InvalidPlayerEntity(error.to_string()))?;
        if entity.user_id != self.user_id || entity.novel_id != self.novel_id {
            return Err(WorldStateError::InvalidPlayerEntity(
                "player scope does not match world state".into(),
            ));
        }
        Ok(Some(entity))
    }

    pub fn open_world(&self) -> Result<Option<WorldSession>, WorldStateError> {
        let root = self
            .state
            .as_object()
            .ok_or(WorldStateError::InvalidObject("root"))?;
        let Some(value) = root.get("open_world") else {
            return Ok(None);
        };
        let session = serde_json::from_value::<WorldSession>(value.clone())
            .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?;
        session
            .validate()
            .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?;
        if session
            .game_rules
            .as_ref()
            .is_some_and(|template| template.novel_id != self.novel_id)
        {
            return Err(WorldStateError::InvalidWorldSession(
                "game rule template belongs to another novel".into(),
            ));
        }
        Ok(Some(session))
    }

    pub fn start_open_world(
        &mut self,
        context: &WorldEntryContext,
    ) -> Result<WorldSession, WorldStateError> {
        self.start_open_world_with_rules(context, None)
    }

    pub fn start_open_world_with_rules(
        &mut self,
        context: &WorldEntryContext,
        game_rules: Option<&GameRuleTemplate>,
    ) -> Result<WorldSession, WorldStateError> {
        context
            .validate()
            .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?;
        let player = self.player_entity()?.ok_or_else(|| {
            WorldStateError::InvalidWorldSession("PlayerEntity must be created first".into())
        })?;
        if player.canonical_checkpoint_chapter != context.checkpoint_chapter {
            return Err(WorldStateError::TimelineConflict(
                "player checkpoint does not match world entry".into(),
            ));
        }
        if let Some(existing) = self.open_world()? {
            // The stored entry context is the pinned canonical checkpoint;
            // a later model must not rewrite it on an idempotent start.
            if existing.game_rules.as_ref() != game_rules {
                return Err(WorldStateError::InvalidWorldSession(
                    "existing world session uses different game rules".into(),
                ));
            }
            return Ok(existing);
        }

        self.validate_world_entry_checkpoint(context.checkpoint_chapter)?;
        let session = WorldSession::from_context_with_rules(context, game_rules)
            .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?;
        let mut next = self.state.clone();
        {
            let threads = object_section(&mut next, "threads")?;
            for thread in &context.threads {
                threads.entry(thread.id.clone()).or_insert_with(|| {
                    serde_json::json!({
                        "status": "open",
                        "description": thread.name,
                        "origin": "canon",
                    })
                });
            }
        }
        next.as_object_mut()
            .ok_or(WorldStateError::InvalidObject("root"))?
            .insert(
                "open_world".into(),
                serde_json::to_value(&session)
                    .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?,
            );
        self.state = next;
        self.updated_at = Utc::now();
        Ok(session)
    }

    pub fn validate_world_entry_checkpoint(
        &self,
        checkpoint_chapter: i32,
    ) -> Result<(), WorldStateError> {
        let choices = self
            .state
            .get("choices")
            .and_then(serde_json::Value::as_array)
            .ok_or(WorldStateError::InvalidChoices)?;
        for choice in choices {
            let chapter = choice
                .get("chapter")
                .and_then(serde_json::Value::as_i64)
                .and_then(|chapter| i32::try_from(chapter).ok())
                .ok_or(WorldStateError::InvalidChoices)?;
            if !(1..=checkpoint_chapter).contains(&chapter) {
                return Err(WorldStateError::TimelineConflict(
                    "world entry checkpoint precedes a committed branch choice".into(),
                ));
            }
        }
        if let Some(events) = self
            .state
            .get("world_events")
            .and_then(serde_json::Value::as_array)
        {
            for event in events {
                let Some(chapter_value) = event.get("chapter") else {
                    continue;
                };
                let chapter = chapter_value
                    .as_i64()
                    .and_then(|chapter| i32::try_from(chapter).ok())
                    .ok_or_else(|| {
                        WorldStateError::InvalidWorldSession(
                            "world event chapter is invalid".into(),
                        )
                    })?;
                if !(1..=checkpoint_chapter).contains(&chapter) {
                    return Err(WorldStateError::TimelineConflict(
                        "world entry checkpoint precedes a committed branch event".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub fn apply_world_turn(
        &mut self,
        turn_id: Uuid,
        action: &WorldAction,
        transition: &WorldTurnTransition,
        context: &WorldEntryContext,
    ) -> Result<(), WorldStateError> {
        self.apply_world_turn_with_check(turn_id, action, transition, context, None)
    }

    pub fn apply_world_turn_with_check(
        &mut self,
        turn_id: Uuid,
        action: &WorldAction,
        transition: &WorldTurnTransition,
        context: &WorldEntryContext,
        resolution: Option<&ActionCheck>,
    ) -> Result<(), WorldStateError> {
        if turn_id.is_nil() {
            return Err(WorldStateError::InvalidWorldSession(
                "world turn ID must not be nil".into(),
            ));
        }
        let mut session = self.open_world()?.ok_or_else(|| {
            WorldStateError::InvalidWorldSession("world session has not started".into())
        })?;
        if transition.prompt_version
            == crate::domain::entities::world_session::WORLD_TURN_PROMPT_VERSION
        {
            self.validate_world_action(action, context)?;
        }
        transition
            .validate_against_with_check(action, context, &session, resolution)
            .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?;
        let mut player = self.player_entity()?.ok_or_else(|| {
            WorldStateError::InvalidWorldSession("PlayerEntity is missing".into())
        })?;

        for removed in &transition.inventory_removals {
            let index = player
                .inventory
                .iter()
                .position(|item| item == removed)
                .ok_or_else(|| {
                    WorldStateError::InvalidWorldSession(format!(
                        "player does not hold inventory item {removed}"
                    ))
                })?;
            player.inventory.remove(index);
        }
        player
            .inventory
            .extend(transition.inventory_additions.iter().cloned());
        player
            .discovered_knowledge
            .extend(transition.knowledge_discoveries.iter().cloned());
        if let Some(location_id) = &transition.player_location_id {
            player.location_id = location_id.clone();
        }
        for change in &transition.relationship_changes {
            let relationship = player
                .relationships
                .entry(change.character_id)
                .or_insert_with(
                    || crate::domain::entities::player_entity::RelationshipState {
                        score: 50,
                        last_change: change.reason.clone(),
                    },
                );
            relationship.score = (relationship.score + change.delta).clamp(0, 100);
            relationship.last_change = change.reason.clone();
            session
                .character_perceptions
                .insert(change.character_id, change.reason.clone());
        }
        for change in &transition.faction_changes {
            let standing = player
                .faction_standing
                .entry(change.faction_id.clone())
                .or_insert(0);
            *standing = (*standing + change.delta).clamp(-100, 100);
        }
        player
            .validate()
            .map_err(|error| WorldStateError::InvalidPlayerEntity(error.to_string()))?;

        let mut canonical_deaths = Vec::new();
        if let Some(index) = session
            .canonical_events
            .iter()
            .position(|event| event.status.is_pending())
        {
            let precondition_failed = session.canonical_events[index]
                .event
                .character_ids
                .iter()
                .any(|id| session.dead_character_ids.contains(id));
            let event = &mut session.canonical_events[index];
            if let Some(change) = &transition.canonical_event_change {
                event.status = change.status;
                event.reason = Some(change.reason.clone());
            } else if precondition_failed {
                event.status = CanonicalEventStatus::Prevented;
                event.reason =
                    Some("canonical precondition failed because an actor is dead".into());
            } else {
                event.status = CanonicalEventStatus::Occurred;
                event.reason = Some("canonical mainline advanced".into());
            }
            if matches!(
                event.status,
                CanonicalEventStatus::Occurred
                    | CanonicalEventStatus::Witnessed
                    | CanonicalEventStatus::Assisted
            ) {
                canonical_deaths = event.event.death_character_ids.clone();
            }
        }
        for character_id in canonical_deaths {
            if !session.dead_character_ids.contains(&character_id) {
                session.dead_character_ids.push(character_id);
            }
        }
        for event in &transition.events {
            for actor in &event.actor_character_ids {
                session
                    .character_perceptions
                    .entry(*actor)
                    .or_insert_with(|| event.summary.clone());
            }
        }
        session.turn_number = session.turn_number.checked_add(1).ok_or_else(|| {
            WorldStateError::InvalidWorldSession("world turn counter overflowed".into())
        })?;
        session.world_time = session
            .world_time
            .checked_add(1)
            .ok_or_else(|| WorldStateError::InvalidWorldSession("world time overflowed".into()))?;
        session
            .validate()
            .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?;

        let mut next = self.state.clone();
        let root = next
            .as_object_mut()
            .ok_or(WorldStateError::InvalidObject("root"))?;
        root.insert(
            "player_entity".into(),
            serde_json::to_value(&player)
                .map_err(|error| WorldStateError::InvalidPlayerEntity(error.to_string()))?,
        );
        root.insert(
            "open_world".into(),
            serde_json::to_value(&session)
                .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?,
        );
        {
            let events = array_section(&mut next, "world_events")?;
            for (index, event) in transition.events.iter().enumerate() {
                events.push(serde_json::json!({
                    "id": format!("{turn_id}:event:{index}"),
                    "origin": "player",
                    "turn_id": turn_id,
                    "turn_number": session.turn_number,
                    "world_time": session.world_time,
                    "summary": event.summary,
                    "actor_character_ids": event.actor_character_ids,
                    "location_id": event.location_id,
                }));
            }
        }
        {
            let locations = object_section(&mut next, "locations")?;
            for change in &transition.location_changes {
                locations.insert(
                    change.location_id.clone(),
                    serde_json::json!({
                        "state": change.state,
                        "reason": change.reason,
                        "origin": "player",
                        "turn_id": turn_id,
                    }),
                );
            }
        }
        {
            let threads = object_section(&mut next, "threads")?;
            for change in &transition.thread_changes {
                threads.insert(
                    change.thread_id.clone(),
                    serde_json::json!({
                        "status": change.status.to_str(),
                        "description": change.description,
                        "origin": "player",
                        "turn_id": turn_id,
                    }),
                );
            }
        }
        self.state = next;
        self.updated_at = Utc::now();
        Ok(())
    }

    pub fn validate_world_action(
        &self,
        action: &WorldAction,
        context: &WorldEntryContext,
    ) -> Result<(), WorldStateError> {
        let session = self.open_world()?.ok_or_else(|| {
            WorldStateError::InvalidWorldSession("world session has not started".into())
        })?;
        if session.entry_context != *context {
            return Err(WorldStateError::InvalidWorldSession(
                "world entry context does not match the current session".into(),
            ));
        }
        self.validate_world_entry_checkpoint(session.entry_context.checkpoint_chapter)?;
        session
            .validate_action(action)
            .map_err(|error| WorldStateError::InvalidWorldSession(error.to_string()))?;
        if matches!(
            action.kind,
            WorldActionKind::AdvanceThread | WorldActionKind::ResolveThread
        ) {
            let target = action.target_id.as_deref().ok_or_else(|| {
                WorldStateError::InvalidWorldSession("thread target is required".into())
            })?;
            let is_open = self
                .state
                .get("threads")
                .and_then(serde_json::Value::as_object)
                .and_then(|threads| threads.get(target))
                .and_then(|thread| thread.get("status"))
                .and_then(serde_json::Value::as_str)
                == Some("open");
            if !is_open {
                return Err(WorldStateError::InvalidWorldSession(
                    "thread target is not open".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn character_world_context(
        &self,
        character_id: Uuid,
    ) -> Result<Option<CharacterWorldContext>, WorldStateError> {
        let Some(session) = self.open_world()? else {
            return Ok(None);
        };
        self.validate_world_entry_checkpoint(session.entry_context.checkpoint_chapter)?;
        if !session
            .entry_context
            .characters
            .iter()
            .any(|character| character.id == character_id)
        {
            return Ok(None);
        }
        let player = self.player_entity()?.ok_or_else(|| {
            WorldStateError::InvalidWorldSession("PlayerEntity is missing".into())
        })?;
        let recent_player_events = self
            .state
            .get("world_events")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .rev()
            .filter_map(parse_world_history_item)
            .filter(|event| event.actor_character_ids.contains(&character_id))
            .map(|event| bounded_world_history_item(event, character_id))
            .take(MAX_CHARACTER_RECENT_EVENTS)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        // Thread rows currently carry no character witness provenance. Keep
        // the wire field but fail closed instead of sharing every open thread.
        let active_threads = vec![];
        let context = CharacterWorldContext {
            user_id: self.user_id,
            novel_id: self.novel_id,
            character_id,
            character_alive: !session.dead_character_ids.contains(&character_id),
            canon_model_version: session.entry_context.model_version,
            checkpoint_chapter: session.entry_context.checkpoint_chapter,
            source_chapter_high_water: session.entry_context.unlocked_through_chapter,
            turn_number: session.turn_number,
            world_time: session.world_time,
            player_id: player.id,
            player_name: player.name,
            player_location_id: player.location_id,
            relationship: player
                .relationships
                .get(&character_id)
                .cloned()
                .map(|mut value| {
                    value.last_change =
                        leading_chars(&value.last_change, MAX_CHARACTER_CONTEXT_TEXT_CHARS);
                    value
                }),
            goals: session
                .entry_context
                .character_goals
                .iter()
                .filter(|goal| goal.character_id == character_id)
                .map(bounded_character_goal)
                .take(MAX_CHARACTER_GOALS)
                .collect(),
            perception_of_player: session
                .character_perceptions
                .get(&character_id)
                .map(|value| leading_chars(value, MAX_CHARACTER_CONTEXT_TEXT_CHARS)),
            current_canonical_event: session
                .canonical_events
                .iter()
                .find(|event| event.status.is_pending())
                .filter(|event| event.event.character_ids.contains(&character_id))
                .map(|event| bounded_canonical_event(event, character_id)),
            recent_actions: vec![],
            recent_player_events,
            active_threads,
        };
        Ok(Some(fit_character_world_context(context)))
    }

    /// 更新角色关系
    pub fn update_relationship(
        &mut self,
        character_name: &str,
        delta: i32,
        reason: &str,
    ) -> Result<(), WorldStateError> {
        self.player_entity()?;
        let relationships = relationship_section(&mut self.state)?;
        let current = relationships
            .get(character_name)
            .and_then(|v| v["score"].as_i64())
            .unwrap_or(50) as i32;

        let new_score = (current + delta).clamp(0, 100);
        relationships.insert(
            character_name.into(),
            serde_json::json!({
                "score": new_score,
                "last_change": reason,
            }),
        );
        self.updated_at = Utc::now();
        Ok(())
    }

    /// 获取与某角色的关系分数（0-100）
    pub fn get_relationship_score(&self, character_name: &str) -> i32 {
        self.state
            .get("player_entity")
            .and_then(|player| player.get("relationships"))
            .or_else(|| self.state.get("relationships"))
            .and_then(|relationships| relationships.get(character_name))
            .and_then(|v| v["score"].as_i64())
            .unwrap_or(50) as i32
    }
}

fn parse_world_history_item(value: &serde_json::Value) -> Option<WorldHistoryItem> {
    let object = value.as_object()?;
    if object.get("origin")?.as_str()? != "player" {
        return None;
    }
    Some(WorldHistoryItem {
        id: object.get("id")?.as_str()?.to_owned(),
        turn_id: Uuid::parse_str(object.get("turn_id")?.as_str()?).ok()?,
        turn_number: object.get("turn_number")?.as_i64()?,
        world_time: object.get("world_time")?.as_i64()?,
        summary: object.get("summary")?.as_str()?.to_owned(),
        actor_character_ids: serde_json::from_value(object.get("actor_character_ids")?.clone())
            .ok()?,
        location_id: object
            .get("location_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    })
}

fn bounded_character_goal(
    goal: &crate::domain::entities::world_session::CharacterGoalRef,
) -> crate::domain::entities::world_session::CharacterGoalRef {
    let mut goal = goal.clone();
    goal.description = leading_chars(&goal.description, MAX_CHARACTER_CONTEXT_TEXT_CHARS);
    keep_recent(
        &mut goal.source_chapters,
        MAX_CHARACTER_CONTEXT_SOURCE_CHAPTERS,
    );
    goal
}

fn bounded_world_history_item(mut event: WorldHistoryItem, character_id: Uuid) -> WorldHistoryItem {
    event.summary = leading_chars(&event.summary, MAX_CHARACTER_CONTEXT_TEXT_CHARS);
    event.actor_character_ids = bounded_character_ids(&event.actor_character_ids, character_id);
    event
}

fn bounded_canonical_event(
    event: &crate::domain::entities::world_session::CanonicalEventState,
    character_id: Uuid,
) -> crate::domain::entities::world_session::CanonicalEventState {
    let mut event = event.clone();
    event.event.summary = leading_chars(&event.event.summary, MAX_CHARACTER_CONTEXT_TEXT_CHARS);
    event.reason = event
        .reason
        .as_deref()
        .map(|reason| leading_chars(reason, MAX_CHARACTER_CONTEXT_TEXT_CHARS));
    event.event.character_ids = bounded_character_ids(&event.event.character_ids, character_id);
    event
        .event
        .location_ids
        .truncate(MAX_CHARACTER_CONTEXT_REFERENCES);
    event
        .event
        .faction_ids
        .truncate(MAX_CHARACTER_CONTEXT_REFERENCES);
    event
        .event
        .death_character_ids
        .retain(|id| event.event.character_ids.contains(id));
    event
        .event
        .death_character_ids
        .truncate(MAX_CHARACTER_CONTEXT_REFERENCES);
    keep_recent(
        &mut event.event.source_chapters,
        MAX_CHARACTER_CONTEXT_SOURCE_CHAPTERS,
    );
    event
}

fn bounded_character_ids(ids: &[Uuid], character_id: Uuid) -> Vec<Uuid> {
    let mut selected = Vec::with_capacity(MAX_CHARACTER_CONTEXT_REFERENCES);
    if ids.contains(&character_id) {
        selected.push(character_id);
    }
    let remaining = MAX_CHARACTER_CONTEXT_REFERENCES.saturating_sub(selected.len());
    selected.extend(
        ids.iter()
            .copied()
            .filter(|id| *id != character_id)
            .take(remaining),
    );
    selected
}

fn keep_recent<T>(values: &mut Vec<T>, maximum: usize) {
    let discard = values.len().saturating_sub(maximum);
    values.drain(..discard);
}

fn leading_chars(value: &str, maximum: usize) -> String {
    value.chars().take(maximum).collect()
}

fn serialized_chars(context: &CharacterWorldContext) -> usize {
    serde_json::to_string(context)
        .map(|value| value.chars().count())
        .unwrap_or(usize::MAX)
}

pub(crate) fn fit_character_world_context(
    mut context: CharacterWorldContext,
) -> CharacterWorldContext {
    while serialized_chars(&context) > MAX_CHARACTER_WORLD_CONTEXT_CHARS {
        if context.recent_player_events.len() > 1 {
            context.recent_player_events.remove(0);
            continue;
        }
        if context.recent_actions.len() > 1 {
            context.recent_actions.remove(0);
            continue;
        }
        if context.perception_of_player.take().is_some() {
            continue;
        }
        if context.relationship.take().is_some() {
            continue;
        }
        if context.active_threads.pop().is_some() {
            continue;
        }
        if context.goals.pop().is_some() {
            continue;
        }
        if context.current_canonical_event.take().is_some() {
            continue;
        }
        if context.recent_player_events.pop().is_some() {
            continue;
        }
        if context.recent_actions.pop().is_some() {
            continue;
        }
        break;
    }
    debug_assert!(serialized_chars(&context) <= MAX_CHARACTER_WORLD_CONTEXT_CHARS);
    context
}

fn relationship_section(
    state: &mut serde_json::Value,
) -> Result<&mut serde_json::Map<String, serde_json::Value>, WorldStateError> {
    let root = state
        .as_object_mut()
        .ok_or(WorldStateError::InvalidObject("root"))?;
    if root
        .get("player_entity")
        .is_some_and(|player| !player.is_null())
    {
        let player = root
            .get_mut("player_entity")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or(WorldStateError::InvalidObject("player_entity"))?;
        player
            .entry("relationships")
            .or_insert_with(|| serde_json::json!({}));
        return player
            .get_mut("relationships")
            .and_then(serde_json::Value::as_object_mut)
            .ok_or(WorldStateError::InvalidObject(
                "player_entity.relationships",
            ));
    }
    root.entry("relationships")
        .or_insert_with(|| serde_json::json!({}));
    root.get_mut("relationships")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or(WorldStateError::InvalidObject("relationships"))
}

fn array_section<'a>(
    state: &'a mut serde_json::Value,
    key: &'static str,
) -> Result<&'a mut Vec<serde_json::Value>, WorldStateError> {
    let root = state
        .as_object_mut()
        .ok_or(WorldStateError::InvalidObject("root"))?;
    root.entry(key).or_insert_with(|| serde_json::json!([]));
    root.get_mut(key)
        .and_then(serde_json::Value::as_array_mut)
        .ok_or(WorldStateError::InvalidArray(key))
}

fn object_section<'a>(
    state: &'a mut serde_json::Value,
    key: &'static str,
) -> Result<&'a mut serde_json::Map<String, serde_json::Value>, WorldStateError> {
    let root = state
        .as_object_mut()
        .ok_or(WorldStateError::InvalidObject("root"))?;
    root.entry(key).or_insert_with(|| serde_json::json!({}));
    root.get_mut(key)
        .and_then(serde_json::Value::as_object_mut)
        .ok_or(WorldStateError::InvalidObject(key))
}

#[cfg(test)]
mod causality_tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_changes_with_prompt_visible_state() {
        let mut state = WorldState::new(Uuid::new_v4(), Uuid::new_v4());
        let original = state.fingerprint();
        assert_eq!(state.fingerprint(), original);

        state.state["reader_reputation"]["watch"] = serde_json::json!(1);
        assert_ne!(state.fingerprint(), original);
    }

    #[test]
    fn source_high_water_tracks_the_latest_committed_choice() {
        let mut state = WorldState::new(Uuid::new_v4(), Uuid::new_v4());
        state.state["choices"] = serde_json::json!([
            {"chapter": 1},
            {"chapter": 4}
        ]);

        assert_eq!(state.latest_choice_chapter().unwrap(), Some(4));
        assert_eq!(state.source_chapter_high_water().unwrap(), Some(4));
    }
}
