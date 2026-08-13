use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::entities::player_entity::PlayerEntity;
use crate::domain::entities::world_session::{
    ActiveThread, CanonicalEventStatus, CharacterWorldContext, WorldAction, WorldEntryContext,
    WorldHistoryItem, WorldSession, WorldTurnTransition,
};
use crate::domain::services::narrative_transition::NarrativeTransition;

/// 叙事节点（关键分支点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeNode {
    pub id: Uuid,
    /// None for the immutable canonical timeline, Some for a player's fork.
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
        Ok(Some(session))
    }

    pub fn start_open_world(
        &mut self,
        context: &WorldEntryContext,
    ) -> Result<WorldSession, WorldStateError> {
        let player = self.player_entity()?.ok_or_else(|| {
            WorldStateError::InvalidWorldSession("PlayerEntity must be created first".into())
        })?;
        if player.canonical_checkpoint_chapter != context.checkpoint_chapter {
            return Err(WorldStateError::InvalidWorldSession(
                "player checkpoint does not match world entry".into(),
            ));
        }
        if let Some(existing) = self.open_world()? {
            if existing.entry_context != *context {
                return Err(WorldStateError::InvalidWorldSession(
                    "existing world session uses different canon".into(),
                ));
            }
            return Ok(existing);
        }

        let session = WorldSession::from_context(context)
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

    pub fn apply_world_turn(
        &mut self,
        turn_id: Uuid,
        action: &WorldAction,
        transition: &WorldTurnTransition,
        context: &WorldEntryContext,
    ) -> Result<(), WorldStateError> {
        if turn_id.is_nil() {
            return Err(WorldStateError::InvalidWorldSession(
                "world turn ID must not be nil".into(),
            ));
        }
        let mut session = self.open_world()?.ok_or_else(|| {
            WorldStateError::InvalidWorldSession("world session has not started".into())
        })?;
        transition
            .validate_against(action, context, &session)
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

    pub fn character_world_context(
        &self,
        character_id: Uuid,
    ) -> Result<Option<CharacterWorldContext>, WorldStateError> {
        let Some(session) = self.open_world()? else {
            return Ok(None);
        };
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
            .take(16)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        let active_threads = self
            .state
            .get("threads")
            .and_then(serde_json::Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(_, value)| {
                value.get("status").and_then(serde_json::Value::as_str) == Some("open")
            })
            .map(|(id, value)| ActiveThread {
                id: id.clone(),
                description: value
                    .get("description")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                origin: value
                    .get("origin")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("canon")
                    .to_owned(),
            })
            .take(32)
            .collect();
        Ok(Some(CharacterWorldContext {
            user_id: self.user_id,
            novel_id: self.novel_id,
            character_id,
            character_alive: !session.dead_character_ids.contains(&character_id),
            canon_model_version: session.entry_context.model_version,
            checkpoint_chapter: session.entry_context.checkpoint_chapter,
            turn_number: session.turn_number,
            world_time: session.world_time,
            player_id: player.id,
            player_name: player.name,
            player_location_id: player.location_id,
            relationship: player.relationships.get(&character_id).cloned(),
            goals: session
                .entry_context
                .character_goals
                .iter()
                .filter(|goal| goal.character_id == character_id)
                .cloned()
                .collect(),
            perception_of_player: session.character_perceptions.get(&character_id).cloned(),
            current_canonical_event: session
                .canonical_events
                .iter()
                .find(|event| event.status.is_pending())
                .filter(|event| event.event.character_ids.contains(&character_id))
                .cloned(),
            recent_player_events,
            active_threads,
        }))
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
