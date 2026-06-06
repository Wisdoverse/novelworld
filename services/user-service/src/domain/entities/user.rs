use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    pub role: UserRole,
    pub email_verified: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_sign_in: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum UserRole {
    User,
    Admin,
}

impl UserRole {
    pub fn as_str(&self) -> &str {
        match self {
            UserRole::User => "user",
            UserRole::Admin => "admin",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "admin" => Self::Admin,
            _ => Self::User,
        }
    }
}

impl User {
    pub fn new(email: String, password_hash: String, name: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            email,
            password_hash,
            name,
            avatar_url: None,
            role: UserRole::User,
            email_verified: false,
            created_at: now,
            updated_at: now,
            last_sign_in: None,
        }
    }

    pub fn record_sign_in(&mut self) {
        self.last_sign_in = Some(Utc::now());
        self.updated_at = Utc::now();
    }
}

#[derive(Debug, Clone)]
pub struct RefreshToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

impl RefreshToken {
    pub fn new(user_id: Uuid, token: String, expires_in_secs: i64) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id,
            token,
            expires_at: now + chrono::Duration::seconds(expires_in_secs),
            created_at: now,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }
}

/// 阅读进度
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadingProgress {
    pub id: Uuid,
    pub user_id: Uuid,
    pub novel_id: Uuid,
    pub current_chapter: i32,
    pub reader_identity: Option<String>,
    pub reader_identity_type: IdentityType,
    pub reader_character_id: Option<Uuid>,
    pub deviation_mode: String,
    pub last_read_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum IdentityType {
    /// 以自己身份进入
    Self_,
    /// 扮演某个角色
    Character,
}

impl ReadingProgress {
    pub fn new(user_id: Uuid, novel_id: Uuid) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            user_id,
            novel_id,
            current_chapter: 1,
            reader_identity: None,
            reader_identity_type: IdentityType::Self_,
            reader_character_id: None,
            deviation_mode: "canon".into(),
            last_read_at: now,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn advance_chapter(&mut self, chapter: i32) {
        if chapter > self.current_chapter {
            self.current_chapter = chapter;
            self.last_read_at = Utc::now();
            self.updated_at = Utc::now();
        }
    }

    pub fn set_identity(
        &mut self,
        identity_type: IdentityType,
        identity_name: Option<String>,
        character_id: Option<Uuid>,
    ) {
        self.reader_identity_type = identity_type;
        self.reader_identity = identity_name;
        self.reader_character_id = character_id;
        self.updated_at = Utc::now();
    }
}
