use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "varchar", rename_all = "snake_case")]
pub enum NovelStatus {
    Pending,
    Parsing,
    Ready,
    Error,
}

/// 故事偏离度（借鉴 KathaaVerse）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "varchar", rename_all = "snake_case")]
pub enum DeviationMode {
    /// 忠实原著
    Canon,
    /// 创意扩展
    Creative,
    /// 自由改写
    Remix,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "varchar", rename_all = "snake_case")]
pub enum CharacterRole {
    Protagonist,
    Antagonist,
    Supporting,
    Minor,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "varchar", rename_all = "snake_case")]
pub enum AvatarStatus {
    Pending,
    Generating,
    Ready,
    Error,
}

impl CharacterRole {
    pub fn to_str(&self) -> &str {
        match self {
            CharacterRole::Protagonist => "protagonist",
            CharacterRole::Antagonist => "antagonist",
            CharacterRole::Supporting => "supporting",
            CharacterRole::Minor => "minor",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "protagonist" => Self::Protagonist,
            "antagonist" => Self::Antagonist,
            "minor" => Self::Minor,
            _ => Self::Supporting,
        }
    }
}

impl AvatarStatus {
    pub fn to_str(&self) -> &str {
        match self {
            AvatarStatus::Pending => "pending",
            AvatarStatus::Generating => "generating",
            AvatarStatus::Ready => "ready",
            AvatarStatus::Error => "error",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "generating" => Self::Generating,
            "ready" => Self::Ready,
            "error" => Self::Error,
            _ => Self::Pending,
        }
    }
}

impl NovelStatus {
    pub fn to_str(&self) -> &str {
        match self {
            NovelStatus::Pending => "pending",
            NovelStatus::Parsing => "parsing",
            NovelStatus::Ready => "ready",
            NovelStatus::Error => "error",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "parsing" => Self::Parsing,
            "ready" => Self::Ready,
            "error" => Self::Error,
            _ => Self::Pending,
        }
    }
}

impl DeviationMode {
    pub fn to_str(&self) -> &str {
        match self {
            DeviationMode::Canon => "canon",
            DeviationMode::Creative => "creative",
            DeviationMode::Remix => "remix",
        }
    }
    pub fn from_str(s: &str) -> Self {
        match s {
            "creative" => Self::Creative,
            "remix" => Self::Remix,
            _ => Self::Canon,
        }
    }
}

/// 读者身份类型
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ReaderIdentityType {
    /// 以自己身份进入
    Self_,
    /// 扮演某个角色
    Character,
}
