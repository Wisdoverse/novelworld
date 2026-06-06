use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::value_objects::{NovelStatus, DeviationMode};
use crate::domain::events::NovelEvent;

/// 小说聚合根
/// DDD: 聚合根负责维护不变量，所有状态变更通过方法进行
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Novel {
    pub id: Uuid,
    pub user_id: Uuid,
    pub title: String,
    pub author: Option<String>,
    pub cover_url: Option<String>,
    pub description: Option<String>,
    pub world_summary: Option<String>,
    pub genre: Option<String>,
    pub file_key: Option<String>,
    pub total_chapters: i32,
    pub status: NovelStatus,
    pub parse_error: Option<String>,
    pub deviation_mode: DeviationMode,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,

    #[serde(skip)]
    pub domain_events: Vec<NovelEvent>,
}

impl Novel {
    /// 工厂方法：创建新小说（pending 状态）
    pub fn create(user_id: Uuid, title: String, author: Option<String>) -> Self {
        let now = Utc::now();
        let mut novel = Self {
            id: Uuid::new_v4(),
            user_id,
            title,
            author,
            cover_url: None,
            description: None,
            world_summary: None,
            genre: None,
            file_key: None,
            total_chapters: 0,
            status: NovelStatus::Pending,
            parse_error: None,
            deviation_mode: DeviationMode::Canon,
            created_at: now,
            updated_at: now,
            domain_events: vec![],
        };
        novel.domain_events.push(NovelEvent::Created {
            novel_id: novel.id,
            user_id,
        });
        novel
    }

    /// 开始解析
    pub fn start_parsing(&mut self) {
        self.status = NovelStatus::Parsing;
        self.updated_at = Utc::now();
    }

    /// 解析完成
    pub fn mark_ready(&mut self, total_chapters: i32, world_summary: String) {
        self.status = NovelStatus::Ready;
        self.total_chapters = total_chapters;
        self.world_summary = Some(world_summary);
        self.updated_at = Utc::now();
        self.domain_events.push(NovelEvent::ParseCompleted {
            novel_id: self.id,
            total_chapters,
        });
    }

    /// 解析失败
    pub fn mark_error(&mut self, error: String) {
        self.status = NovelStatus::Error;
        self.parse_error = Some(error.clone());
        self.updated_at = Utc::now();
        self.domain_events.push(NovelEvent::ParseFailed {
            novel_id: self.id,
            error,
        });
    }

    /// 设置故事偏离度
    pub fn set_deviation_mode(&mut self, mode: DeviationMode) {
        self.deviation_mode = mode;
        self.updated_at = Utc::now();
    }

    /// 取出并清空领域事件
    pub fn take_events(&mut self) -> Vec<NovelEvent> {
        std::mem::take(&mut self.domain_events)
    }
}
