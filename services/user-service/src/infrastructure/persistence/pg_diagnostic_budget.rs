use std::{future::Future, time::Duration};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::domain::{
    entities::diagnostic_budget::{
        Amount, Attempt, BudgetError, BudgetSnapshot, Profile, Registration, Reservation,
        Settlement,
    },
    repositories::diagnostic_budget::DiagnosticBudgetRepository,
};

pub struct PgDiagnosticBudgetRepository {
    pool: PgPool,
}

impl PgDiagnosticBudgetRepository {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn transaction(&self) -> Result<Transaction<'_, Postgres>, BudgetError> {
        let mut tx = self.pool.begin().await.map_err(database_error)?;
        // No retry: a lost commit acknowledgement is an unknown, possibly committed outcome.
        // Server deadlines also bound abandoned SQL when the outer future is cancelled.
        sqlx::query_as::<_, TransactionTimeouts>(
            "SELECT pg_catalog.set_config('statement_timeout', '2s', true) AS _statement_timeout,
             pg_catalog.set_config('lock_timeout', '1s', true) AS _lock_timeout",
        )
        .fetch_one(&mut *tx)
        .await
        .map_err(database_error)?;
        Ok(tx)
    }
}

#[derive(sqlx::FromRow)]
struct TransactionTimeouts {
    _statement_timeout: String,
    _lock_timeout: String,
}

async fn bounded<T>(work: impl Future<Output = Result<T, BudgetError>>) -> Result<T, BudgetError> {
    tokio::time::timeout(Duration::from_secs(4), work)
        .await
        .map_err(|_| BudgetError::Unavailable)?
}

fn database_error(error: sqlx::Error) -> BudgetError {
    if error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
    {
        BudgetError::Conflict
    } else {
        // Do not expose SQL, bound values, or infrastructure diagnostics on this control path.
        BudgetError::Unavailable
    }
}

#[derive(sqlx::FromRow)]
struct DatabaseClock {
    now: DateTime<Utc>,
}

async fn database_now(tx: &mut Transaction<'_, Postgres>) -> Result<DateTime<Utc>, BudgetError> {
    Ok(
        sqlx::query_as::<_, DatabaseClock>("SELECT clock_timestamp() AS now")
            .fetch_one(&mut **tx)
            .await
            .map_err(database_error)?
            .now,
    )
}

#[derive(sqlx::FromRow)]
struct BudgetRow {
    budget_id: Uuid,
    contract: String,
    profile: String,
    profile_sha256: String,
    max_attempts: i64,
    max_tokens: i64,
    max_cost_micro_cny: i64,
    charged_attempts: i64,
    charged_tokens: i64,
    charged_cost_micro_cny: i64,
    expires_at: DateTime<Utc>,
    sealed: bool,
}

fn unsigned(value: i64) -> Result<u64, BudgetError> {
    u64::try_from(value).map_err(|_| BudgetError::Invalid)
}

fn signed(value: u64) -> Result<i64, BudgetError> {
    i64::try_from(value).map_err(|_| BudgetError::Invalid)
}

fn validate_registration(registration: &Registration) -> Result<(), BudgetError> {
    registration.validate()?;
    // Hashing stays in infrastructure, but every durable entrypoint enforces the compiled binding.
    if registration.profile_sha256 != llm_client::diagnostic_budget::profile_sha256() {
        return Err(BudgetError::Invalid);
    }
    Ok(())
}

impl BudgetRow {
    fn verify(self, expected: &Registration) -> Result<BudgetSnapshot, BudgetError> {
        let registration = Registration {
            budget_id: self.budget_id,
            contract: self.contract,
            profile: self.profile,
            profile_sha256: self.profile_sha256,
            limits: Amount {
                attempts: unsigned(self.max_attempts)?,
                tokens: unsigned(self.max_tokens)?,
                cost_micro_cny: unsigned(self.max_cost_micro_cny)?,
            },
            expires_at: self.expires_at,
        };
        if registration != *expected {
            return Err(BudgetError::Conflict);
        }
        let charged = Amount {
            attempts: unsigned(self.charged_attempts)?,
            tokens: unsigned(self.charged_tokens)?,
            cost_micro_cny: unsigned(self.charged_cost_micro_cny)?,
        };
        if !charged.within(registration.limits) {
            return Err(BudgetError::Invalid);
        }
        Ok(BudgetSnapshot {
            registration,
            charged,
            sealed: self.sealed,
        })
    }
}

async fn locked_budget(
    tx: &mut Transaction<'_, Postgres>,
    registration: &Registration,
) -> Result<BudgetSnapshot, BudgetError> {
    validate_registration(registration)?;
    sqlx::query_as::<_, BudgetRow>(
        "SELECT * FROM diagnostic_llm_budgets WHERE budget_id = $1 FOR UPDATE",
    )
    .bind(registration.budget_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(database_error)?
    .ok_or(BudgetError::NotFound)?
    .verify(registration)
}

async fn charge(
    tx: &mut Transaction<'_, Postgres>,
    budget_id: Uuid,
    amount: Amount,
) -> Result<(), BudgetError> {
    sqlx::query(
        "UPDATE diagnostic_llm_budgets SET charged_attempts = $2, charged_tokens = $3,
         charged_cost_micro_cny = $4 WHERE budget_id = $1",
    )
    .bind(budget_id)
    .bind(signed(amount.attempts)?)
    .bind(signed(amount.tokens)?)
    .bind(signed(amount.cost_micro_cny)?)
    .execute(&mut **tx)
    .await
    .map_err(database_error)?;
    Ok(())
}

#[derive(sqlx::FromRow)]
struct AttemptRow {
    operation: String,
    output_limit: i32,
    reservation_tokens: i64,
    reservation_cost_micro_cny: i64,
    settled: bool,
    settlement_model: Option<String>,
    input_tokens: Option<i64>,
    output_tokens: Option<i64>,
    cached_input_tokens: Option<i64>,
}

impl AttemptRow {
    fn settlement(&self) -> Result<Option<Settlement>, BudgetError> {
        if !self.settled {
            return Ok(None);
        }
        Ok(Some(Settlement {
            model: self.settlement_model.clone().ok_or(BudgetError::Invalid)?,
            input_tokens: unsigned(self.input_tokens.ok_or(BudgetError::Invalid)?)?,
            output_tokens: unsigned(self.output_tokens.ok_or(BudgetError::Invalid)?)?,
            cached_input_tokens: self.cached_input_tokens.map(unsigned).transpose()?,
        }))
    }
}

#[async_trait]
impl DiagnosticBudgetRepository for PgDiagnosticBudgetRepository {
    async fn provision(&self, registration: &Registration) -> Result<(), BudgetError> {
        bounded(async {
            validate_registration(registration)?;
            let mut tx = self.transaction().await?;
            registration.validate_provision(database_now(&mut tx).await?)?;
            sqlx::query(
                "INSERT INTO diagnostic_llm_budgets
                 (budget_id, contract, profile, profile_sha256, max_attempts, max_tokens,
                  max_cost_micro_cny, expires_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(registration.budget_id)
            .bind(&registration.contract)
            .bind(&registration.profile)
            .bind(&registration.profile_sha256)
            .bind(signed(registration.limits.attempts)?)
            .bind(signed(registration.limits.tokens)?)
            .bind(signed(registration.limits.cost_micro_cny)?)
            .bind(registration.expires_at)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            tx.commit().await.map_err(database_error)
        })
        .await
    }

    async fn read(&self, registration: &Registration) -> Result<BudgetSnapshot, BudgetError> {
        bounded(async {
            validate_registration(registration)?;
            let mut tx = self.transaction().await?;
            let snapshot = sqlx::query_as::<_, BudgetRow>(
                "SELECT * FROM diagnostic_llm_budgets WHERE budget_id = $1",
            )
            .bind(registration.budget_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or(BudgetError::NotFound)?
            .verify(registration)?;
            tx.commit().await.map_err(database_error)?;
            Ok(snapshot)
        })
        .await
    }

    async fn reserve(
        &self,
        registration: &Registration,
        attempt: &Attempt,
    ) -> Result<Reservation, BudgetError> {
        bounded(async {
            let quote = attempt.quote()?;
            let mut tx = self.transaction().await?;
            let snapshot = locked_budget(&mut tx, registration).await?;
            if sqlx::query_as::<_, AttemptRow>(
                "SELECT * FROM diagnostic_llm_attempts WHERE budget_id = $1 AND attempt_id = $2",
            )
            .bind(registration.budget_id)
            .bind(attempt.attempt_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .is_some()
            {
                return Err(BudgetError::Conflict);
            }
            // Read wall time AFTER lock acquisition, not from a SELECT evaluated before its wait.
            let charged = snapshot.reserve(quote, database_now(&mut tx).await?)?;
            sqlx::query(
                "INSERT INTO diagnostic_llm_attempts
                 (budget_id, attempt_id, ordinal, operation, output_limit,
                  reservation_tokens, reservation_cost_micro_cny)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(registration.budget_id)
            .bind(attempt.attempt_id)
            .bind(signed(charged.attempts)?)
            .bind(&attempt.operation)
            .bind(i32::try_from(attempt.output_limit).map_err(|_| BudgetError::Invalid)?)
            .bind(signed(quote.tokens)?)
            .bind(signed(quote.cost_micro_cny)?)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            charge(&mut tx, registration.budget_id, charged).await?;
            tx.commit().await.map_err(database_error)?;
            Ok(Reservation {
                attempt_id: attempt.attempt_id,
                ordinal: charged.attempts,
                amount: quote,
            })
        })
        .await
    }

    async fn settle(
        &self,
        registration: &Registration,
        attempt_id: Uuid,
        settlement: &Settlement,
    ) -> Result<(), BudgetError> {
        bounded(async {
            let mut tx = self.transaction().await?;
            let snapshot = locked_budget(&mut tx, registration).await?;
            let receipt = sqlx::query_as::<_, AttemptRow>(
                "SELECT * FROM diagnostic_llm_attempts WHERE budget_id = $1 AND attempt_id = $2",
            )
            .bind(registration.budget_id)
            .bind(attempt_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(database_error)?
            .ok_or(BudgetError::NotFound)?;
            if let Some(previous) = receipt.settlement()? {
                if previous != *settlement {
                    return Err(BudgetError::Conflict);
                }
                return tx.commit().await.map_err(database_error);
            }
            let profile = Profile::compiled();
            let output_limit =
                u32::try_from(receipt.output_limit).map_err(|_| BudgetError::Invalid)?;
            let reserved = Amount {
                attempts: 1,
                tokens: unsigned(receipt.reservation_tokens)?,
                cost_micro_cny: unsigned(receipt.reservation_cost_micro_cny)?,
            };
            if reserved != profile.quote(&receipt.operation, output_limit)? {
                return Err(BudgetError::Invalid);
            }
            let actual = profile.usage(&receipt.operation, output_limit, settlement)?;
            let charged = snapshot
                .charged
                .checked_sub(reserved)?
                .checked_add(actual)?;
            sqlx::query(
                "UPDATE diagnostic_llm_attempts SET settled = true, settlement_model = $3,
                 input_tokens = $4, output_tokens = $5, cached_input_tokens = $6
                 WHERE budget_id = $1 AND attempt_id = $2",
            )
            .bind(registration.budget_id)
            .bind(attempt_id)
            .bind(&settlement.model)
            .bind(signed(settlement.input_tokens)?)
            .bind(signed(settlement.output_tokens)?)
            .bind(settlement.cached_input_tokens.map(signed).transpose()?)
            .execute(&mut *tx)
            .await
            .map_err(database_error)?;
            charge(&mut tx, registration.budget_id, charged).await?;
            tx.commit().await.map_err(database_error)
        })
        .await
    }

    async fn seal(&self, registration: &Registration) -> Result<(), BudgetError> {
        bounded(async {
            let mut tx = self.transaction().await?;
            locked_budget(&mut tx, registration).await?;
            sqlx::query("UPDATE diagnostic_llm_budgets SET sealed = true WHERE budget_id = $1")
                .bind(registration.budget_id)
                .execute(&mut *tx)
                .await
                .map_err(database_error)?;
            tx.commit().await.map_err(database_error)
        })
        .await
    }
}
