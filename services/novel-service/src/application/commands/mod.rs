use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::domain::value_objects::DeviationMode;

/// 导入小说命令
#[derive(Debug)]
pub struct ImportNovelCommand {
    pub user_id: Uuid,
    pub title: String,
    pub author: Option<String>,
    /// 原始文本内容（粘贴方式）
    pub raw_content: Option<String>,
    /// 上传文件原始字节；粘贴导入不包含此字段。
    pub source_bytes: Option<bytes::Bytes>,
    pub deviation_mode: Option<DeviationMode>,
}

/// 生成角色头像命令
#[derive(Debug, Serialize, Deserialize)]
pub struct GenerateAvatarCommand {
    pub character_id: Uuid,
    pub novel_id: Uuid,
}

/// 更新偏离度命令
#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateDeviationModeCommand {
    pub novel_id: Uuid,
    pub user_id: Uuid,
    pub mode: DeviationMode,
}
