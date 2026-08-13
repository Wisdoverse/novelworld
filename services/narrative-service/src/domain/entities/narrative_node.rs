use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
            let relationships = object_section(&mut next, "relationships")?;
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

    /// 更新角色关系
    pub fn update_relationship(&mut self, character_name: &str, delta: i32, reason: &str) {
        let current = self.state["relationships"]
            .get(character_name)
            .and_then(|v| v["score"].as_i64())
            .unwrap_or(50) as i32;

        let new_score = (current + delta).clamp(0, 100);
        self.state["relationships"][character_name] = serde_json::json!({
            "score": new_score,
            "last_change": reason,
        });
        self.updated_at = Utc::now();
    }

    /// 获取与某角色的关系分数（0-100）
    pub fn get_relationship_score(&self, character_name: &str) -> i32 {
        self.state["relationships"]
            .get(character_name)
            .and_then(|v| v["score"].as_i64())
            .unwrap_or(50) as i32
    }
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
