use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;
use anyhow::Result;

use crate::domain::entities::user::{User, UserRole, RefreshToken};
use crate::domain::repositories::UserRepository;

pub struct PgUserRepository {
    pool: PgPool,
}

impl PgUserRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserRepository for PgUserRepository {
    async fn save(&self, user: &User) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO users (id, email, password_hash, name, avatar_url, role, email_verified, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)"#
        )
        .bind(user.id)
        .bind(&user.email)
        .bind(&user.password_hash)
        .bind(&user.name)
        .bind(&user.avatar_url)
        .bind(user.role.as_str())
        .bind(user.email_verified)
        .bind(user.created_at)
        .bind(user.updated_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_by_id(&self, id: Uuid) -> Result<Option<User>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, email, password_hash, name, avatar_url, role::text, email_verified, created_at, updated_at, last_sign_in FROM users WHERE id = $1"
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.into()))
    }

    async fn find_by_email(&self, email: &str) -> Result<Option<User>> {
        let row = sqlx::query_as::<_, UserRow>(
            "SELECT id, email, password_hash, name, avatar_url, role::text, email_verified, created_at, updated_at, last_sign_in FROM users WHERE LOWER(email) = LOWER($1)"
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.into()))
    }

    async fn update(&self, user: &User) -> Result<()> {
        sqlx::query(
            r#"UPDATE users SET name=$2, avatar_url=$3, role=$4, email_verified=$5, updated_at=$6, last_sign_in=$7 WHERE id=$1"#
        )
        .bind(user.id)
        .bind(&user.name)
        .bind(&user.avatar_url)
        .bind(user.role.as_str())
        .bind(user.email_verified)
        .bind(user.updated_at)
        .bind(user.last_sign_in)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn save_refresh_token(&self, token: &RefreshToken) -> Result<()> {
        sqlx::query(
            r#"INSERT INTO refresh_tokens (id, user_id, token, expires_at, created_at)
               VALUES ($1, $2, $3, $4, $5)"#
        )
        .bind(token.id)
        .bind(token.user_id)
        .bind(&token.token)
        .bind(token.expires_at)
        .bind(token.created_at)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn find_refresh_token(&self, token: &str) -> Result<Option<RefreshToken>> {
        let row = sqlx::query_as::<_, RefreshTokenRow>(
            "SELECT id, user_id, token, expires_at, created_at FROM refresh_tokens WHERE token = $1"
        )
        .bind(token)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| RefreshToken {
            id: r.id,
            user_id: r.user_id,
            token: r.token,
            expires_at: r.expires_at,
            created_at: r.created_at,
        }))
    }

    async fn delete_refresh_token(&self, token: &str) -> Result<()> {
        sqlx::query("DELETE FROM refresh_tokens WHERE token = $1")
            .bind(token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn delete_refresh_tokens_for_user(&self, user_id: Uuid) -> Result<()> {
        sqlx::query("DELETE FROM refresh_tokens WHERE user_id = $1")
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: Uuid,
    email: String,
    password_hash: String,
    name: Option<String>,
    avatar_url: Option<String>,
    role: String,
    email_verified: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    last_sign_in: Option<chrono::DateTime<chrono::Utc>>,
}

impl From<UserRow> for User {
    fn from(r: UserRow) -> Self {
        User {
            id: r.id,
            email: r.email,
            password_hash: r.password_hash,
            name: r.name,
            avatar_url: r.avatar_url,
            role: UserRole::from_str(&r.role),
            email_verified: r.email_verified,
            created_at: r.created_at,
            updated_at: r.updated_at,
            last_sign_in: r.last_sign_in,
        }
    }
}

#[derive(sqlx::FromRow)]
struct RefreshTokenRow {
    id: Uuid,
    user_id: Uuid,
    token: String,
    expires_at: chrono::DateTime<chrono::Utc>,
    created_at: chrono::DateTime<chrono::Utc>,
}
