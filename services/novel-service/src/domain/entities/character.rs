use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::value_objects::{CharacterRole, AvatarStatus};

/// 角色实体
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Character {
    pub id: Uuid,
    pub novel_id: Uuid,
    pub name: String,
    pub aliases: Vec<String>,
    pub role: CharacterRole,
    pub description: Option<String>,
    pub personality: Option<String>,
    pub background: Option<String>,
    pub speaking_style: Option<String>,
    pub appearance: Option<String>,
    pub avatar_url: Option<String>,
    pub avatar_status: AvatarStatus,
    /// Agent 系统提示词（由 AI 根据角色信息自动生成）
    pub system_prompt: Option<String>,
    pub first_appearance_chapter: Option<i32>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Character {
    pub fn new(novel_id: Uuid, name: String, role: CharacterRole) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            novel_id,
            name,
            aliases: vec![],
            role,
            description: None,
            personality: None,
            background: None,
            speaking_style: None,
            appearance: None,
            avatar_url: None,
            avatar_status: AvatarStatus::Pending,
            system_prompt: None,
            first_appearance_chapter: None,
            created_at: now,
            updated_at: now,
        }
    }

    /// 构建 Agent 系统提示词
    pub fn build_system_prompt(&mut self, novel_title: &str, world_summary: &str) {
        let personality = self.personality.as_deref().unwrap_or("未知");
        let background = self.background.as_deref().unwrap_or("未知");
        let speaking_style = self.speaking_style.as_deref().unwrap_or("自然");
        let description = self.description.as_deref().unwrap_or("");

        self.system_prompt = Some(format!(
            r#"你是《{novel_title}》中的角色「{name}」。

## 世界观背景
{world_summary}

## 你的角色信息
- **姓名**：{name}
- **描述**：{description}
- **性格特征**：{personality}
- **背景故事**：{background}
- **说话风格**：{speaking_style}

## 行为准则
1. 始终以「{name}」的身份和视角回应，保持角色一致性
2. 说话风格和用词习惯要符合角色设定
3. 只知道你在故事中已经经历过的事情（防剧透原则）
4. 记住与读者的历史对话，保持关系的连续性
5. 回应要自然、沉浸，不要打破第四堵墙
6. 如果读者问到你不应该知道的信息，以角色视角自然回避"#,
            novel_title = novel_title,
            name = self.name,
            world_summary = world_summary,
            description = description,
            personality = personality,
            background = background,
            speaking_style = speaking_style,
        ));
        self.updated_at = Utc::now();
    }

    pub fn set_avatar(&mut self, url: String) {
        self.avatar_url = Some(url);
        self.avatar_status = AvatarStatus::Ready;
        self.updated_at = Utc::now();
    }
}
