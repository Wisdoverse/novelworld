use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::Semaphore;

use crate::domain::ports::{PasswordHasher, PasswordHasherError};

const BCRYPT_COST: u32 = 12;

pub struct BcryptPasswordHasher {
    permits: Arc<Semaphore>,
}

impl BcryptPasswordHasher {
    pub fn new(max_in_flight: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(max_in_flight)),
        }
    }
}

#[async_trait]
impl PasswordHasher for BcryptPasswordHasher {
    async fn hash(&self, password: &str) -> Result<String, PasswordHasherError> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| PasswordHasherError::Capacity)?;
        let password = password.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            bcrypt::hash(password, BCRYPT_COST)
        })
        .await
        .map_err(|error| PasswordHasherError::Internal(error.into()))?
        .map_err(|error| PasswordHasherError::Internal(error.into()))
    }

    async fn verify(&self, password: &str, hash: &str) -> Result<bool, PasswordHasherError> {
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| PasswordHasherError::Capacity)?;
        let password = password.to_owned();
        let hash = hash.to_owned();
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            bcrypt::verify(password, &hash)
        })
        .await
        .map_err(|error| PasswordHasherError::Internal(error.into()))?
        .map_err(|error| PasswordHasherError::Internal(error.into()))
    }
}
