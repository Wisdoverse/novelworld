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
