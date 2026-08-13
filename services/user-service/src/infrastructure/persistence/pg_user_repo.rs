use aes_gcm::{
    aead::{Aead, Generate, KeyInit, Nonce, Payload},
    Aes256Gcm,
};
use anyhow::Result;
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;

use crate::domain::entities::runtime_config::RuntimeLlmConfig;
use crate::domain::entities::user::{RefreshToken, User, UserRole};
use crate::domain::repositories::{AccountDeletion, UserRepository, UserSave};

const CONFIG_AAD: &[u8] = b"novelworld-runtime-llm-v1";

pub struct PgUserRepository {
    pool: PgPool,
    cipher: Aes256Gcm,
}

impl PgUserRepository {
    pub fn new(pool: PgPool, encryption_key: &str) -> Result<Self> {
        let key = decode_hex_key(encryption_key)?;
        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| anyhow::anyhow!("RUNTIME_CONFIG_KEY must contain 32 bytes"))?;
        Ok(Self { pool, cipher })
    }

    fn encrypt_api_key(&self, api_key: &str) -> Result<(Vec<u8>, Vec<u8>)> {
        let nonce = Nonce::<Aes256Gcm>::try_generate()
            .map_err(|_| anyhow::anyhow!("failed to generate runtime configuration nonce"))?;
        let ciphertext = self
            .cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: api_key.as_bytes(),
                    aad: CONFIG_AAD,
                },
            )
            .map_err(|_| anyhow::anyhow!("failed to encrypt runtime configuration"))?;
        Ok((nonce.to_vec(), ciphertext))
    }

    fn decrypt_api_key(&self, nonce: &[u8], ciphertext: &[u8]) -> Result<String> {
        let nonce: &[u8; 12] = nonce
            .try_into()
            .map_err(|_| anyhow::anyhow!("runtime configuration nonce is invalid"))?;
        let plaintext = self
            .cipher
            .decrypt(
                nonce.into(),
                Payload {
                    msg: ciphertext,
                    aad: CONFIG_AAD,
                },
            )
            .map_err(|_| anyhow::anyhow!("failed to decrypt runtime configuration"))?;
        String::from_utf8(plaintext)
            .map_err(|_| anyhow::anyhow!("runtime configuration contains invalid UTF-8"))
    }
}

fn decode_hex_key(value: &str) -> Result<[u8; 32]> {
    let value = value.trim();
    if value.len() != 64 {
        return Err(anyhow::anyhow!(
            "RUNTIME_CONFIG_KEY must be 64 hexadecimal characters"
        ));
    }
    let mut key = [0_u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| anyhow::anyhow!("RUNTIME_CONFIG_KEY must be hexadecimal"))?;
    }
    Ok(key)
}

#[async_trait]
impl UserRepository for PgUserRepository {
    async fn save(&self, user: &User) -> Result<UserSave> {
        let mut transaction = self.pool.begin().await?;
        // Coordinate ordinary registration with final-admin deletion without serializing other inserts.
        sqlx::query("LOCK TABLE users IN ROW EXCLUSIVE MODE")
            .execute(&mut *transaction)
            .await?;
        if user.role != UserRole::Admin {
            let has_admin: bool =
                sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE role = 'admin')")
                    .fetch_one(&mut *transaction)
                    .await?;
            if !has_admin {
                transaction.rollback().await?;
                return Ok(UserSave::SetupRequired);
            }
        }
        let result = sqlx::query(
            r#"INSERT INTO users (id, email, password_hash, name, avatar_url, role, email_verified, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6::public.user_role, $7, $8, $9)
               ON CONFLICT DO NOTHING"#
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
        .execute(&mut *transaction)
        .await?;
        transaction.commit().await?;
        Ok(if result.rows_affected() == 1 {
            UserSave::Saved
        } else {
            UserSave::EmailConflict
        })
    }

    async fn save_initial_setup(
        &self,
        user: &User,
        token: &RefreshToken,
        llm: Option<&RuntimeLlmConfig>,
    ) -> Result<bool> {
        let encrypted_key = llm
            .map(|config| self.encrypt_api_key(&config.api_key))
            .transpose()?;
        let mut transaction = self.pool.begin().await?;
        // ponytail: one global lock for the one-time bootstrap; revisit only if setup throughput matters.
        sqlx::query("LOCK TABLE users IN EXCLUSIVE MODE")
            .execute(&mut *transaction)
            .await?;
        let configured: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users)")
            .fetch_one(&mut *transaction)
            .await?;
        if configured {
            transaction.rollback().await?;
            return Ok(false);
        }

        sqlx::query(
            r#"INSERT INTO users (id, email, password_hash, name, avatar_url, role, email_verified, created_at, updated_at)
               VALUES ($1, $2, $3, $4, $5, $6::public.user_role, $7, $8, $9)"#,
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
        .execute(&mut *transaction)
        .await?;
        sqlx::query(
            "INSERT INTO refresh_tokens (id, user_id, token, expires_at, created_at) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(token.id)
        .bind(token.user_id)
        .bind(&token.token)
        .bind(token.expires_at)
        .bind(token.created_at)
        .execute(&mut *transaction)
        .await?;
        if let (Some(config), Some((nonce, ciphertext))) = (llm, encrypted_key) {
            sqlx::query(
                r#"INSERT INTO runtime_llm_config
                   (singleton, provider, api_url, model, thinking_enabled, api_key_nonce, api_key_ciphertext)
                   VALUES (TRUE, $1, $2, $3, $4, $5, $6)
                   ON CONFLICT (singleton) DO UPDATE SET
                     provider = EXCLUDED.provider,
                     api_url = EXCLUDED.api_url,
                     model = EXCLUDED.model,
                     thinking_enabled = EXCLUDED.thinking_enabled,
                     api_key_nonce = EXCLUDED.api_key_nonce,
                     api_key_ciphertext = EXCLUDED.api_key_ciphertext,
                     updated_at = NOW()"#,
            )
            .bind(&config.provider)
            .bind(&config.api_url)
            .bind(&config.model)
            .bind(config.thinking_enabled)
            .bind(nonce)
            .bind(ciphertext)
            .execute(&mut *transaction)
            .await?;
        }
        transaction.commit().await?;
        Ok(true)
    }

    async fn has_any(&self) -> Result<bool> {
        Ok(sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users)")
            .fetch_one(&self.pool)
            .await?)
    }

    async fn find_runtime_llm_config(&self) -> Result<Option<RuntimeLlmConfig>> {
        let row = sqlx::query_as::<_, RuntimeLlmConfigRow>(
            r#"SELECT provider, api_url, model, thinking_enabled, api_key_nonce, api_key_ciphertext
               FROM runtime_llm_config WHERE singleton = TRUE"#,
        )
        .fetch_optional(&self.pool)
        .await?;
        row.map(|row| {
            Ok(RuntimeLlmConfig {
                provider: row.provider,
                api_url: row.api_url,
                model: row.model,
                thinking_enabled: row.thinking_enabled,
                api_key: self.decrypt_api_key(&row.api_key_nonce, &row.api_key_ciphertext)?,
            })
        })
        .transpose()
    }

    async fn save_runtime_llm_config(&self, config: &RuntimeLlmConfig) -> Result<()> {
        let (nonce, ciphertext) = self.encrypt_api_key(&config.api_key)?;
        sqlx::query(
            r#"INSERT INTO runtime_llm_config
               (singleton, provider, api_url, model, thinking_enabled, api_key_nonce, api_key_ciphertext)
               VALUES (TRUE, $1, $2, $3, $4, $5, $6)
               ON CONFLICT (singleton) DO UPDATE SET
                 provider = EXCLUDED.provider,
                 api_url = EXCLUDED.api_url,
                 model = EXCLUDED.model,
                 thinking_enabled = EXCLUDED.thinking_enabled,
                 api_key_nonce = EXCLUDED.api_key_nonce,
                 api_key_ciphertext = EXCLUDED.api_key_ciphertext,
                 updated_at = NOW()"#,
        )
        .bind(&config.provider)
        .bind(&config.api_url)
        .bind(&config.model)
        .bind(config.thinking_enabled)
        .bind(nonce)
        .bind(ciphertext)
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
            r#"UPDATE users SET name=$2, avatar_url=$3, role=$4::public.user_role, email_verified=$5, updated_at=$6, last_sign_in=$7 WHERE id=$1"#
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
               VALUES ($1, $2, $3, $4, $5)"#,
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

    async fn delete_account(&self, user_id: Uuid) -> Result<AccountDeletion> {
        let mut transaction = self.pool.begin().await?;
        // ponytail: account deletion is rare; a global lock makes the last-admin invariant atomic.
        sqlx::query("LOCK TABLE users IN EXCLUSIVE MODE")
            .execute(&mut *transaction)
            .await?;
        let account = sqlx::query_as::<_, (String, i64, i64)>(
            r#"SELECT role::text,
                      (SELECT COUNT(*) FROM users),
                      (SELECT COUNT(*) FROM users WHERE role = 'admin')
               FROM users WHERE id = $1"#,
        )
        .bind(user_id)
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((role, user_count, admin_count)) = account else {
            transaction.rollback().await?;
            return Ok(AccountDeletion::AlreadyAbsent);
        };
        if role == UserRole::Admin.as_str() && admin_count == 1 && user_count > 1 {
            transaction.rollback().await?;
            return Ok(AccountDeletion::LastAdministrator);
        }

        sqlx::query("DELETE FROM users WHERE id = $1")
            .bind(user_id)
            .execute(&mut *transaction)
            .await?;
        if user_count == 1 {
            sqlx::query("DELETE FROM runtime_llm_config")
                .execute(&mut *transaction)
                .await?;
        }
        transaction.commit().await?;
        Ok(AccountDeletion::Deleted)
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

#[derive(sqlx::FromRow)]
struct RuntimeLlmConfigRow {
    provider: String,
    api_url: String,
    model: String,
    thinking_enabled: bool,
    api_key_nonce: Vec<u8>,
    api_key_ciphertext: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::decode_hex_key;

    #[test]
    fn runtime_key_must_be_exactly_32_hex_bytes() {
        assert!(decode_hex_key(&"ab".repeat(32)).is_ok());
        assert!(decode_hex_key("not-a-key").is_err());
        assert!(decode_hex_key(&"ab".repeat(31)).is_err());
    }
}
