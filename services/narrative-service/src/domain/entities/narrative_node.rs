use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 叙事节点（关键分支点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeNode {
    pub id: Uuid,
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
