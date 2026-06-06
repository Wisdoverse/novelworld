use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 叙事节点（关键分支点）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeNode {
    pub id: Uuid,
    pub novel_id: Uuid,
    pub chapter_id: Uuid,
    pub chapter_number: i32,
    /// 节点描述（触发分支的情境）
    pub description: String,
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
        chapter_id: Uuid,
        chapter_number: i32,
        description: String,
        choices: Vec<NarrativeChoice>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            novel_id,
            chapter_id,
            chapter_number,
            description,
            choices,
            created_at: Utc::now(),
        }
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
        chapter: i32,
        choice_text: &str,
        consequence: &str,
    ) {
        if let Some(choices) = self.state["choices"].as_array_mut() {
            choices.push(serde_json::json!({
                "chapter": chapter,
                "choice": choice_text,
                "consequence": consequence,
                "timestamp": Utc::now().to_rfc3339(),
            }));
        }
        self.updated_at = Utc::now();
    }

    /// 更新角色关系
    pub fn update_relationship(
        &mut self,
        character_name: &str,
        delta: i32,
        reason: &str,
    ) {
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
