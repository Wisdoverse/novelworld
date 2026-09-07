use async_trait::async_trait;
use uuid::Uuid;

use crate::domain::entities::diagnostic_budget::{
    Attempt, BudgetError, BudgetSnapshot, Registration, Reservation, Settlement,
};

/// Sole durable authority for the isolated diagnostic. There is deliberately no reset/delete port.
#[async_trait]
pub trait DiagnosticBudgetRepository: Send + Sync {
    /// Explicit one-shot provisioning only. Existing identities conflict, even if unused.
    async fn provision(&self, registration: &Registration) -> Result<(), BudgetError>;
    /// Read and verify the complete immutable registration, including on ordinary startup.
    async fn read(&self, registration: &Registration) -> Result<BudgetSnapshot, BudgetError>;
    /// Every attempt gets a new grant. Duplicate attempt IDs are never replayable grants.
    async fn reserve(
        &self,
        registration: &Registration,
        attempt: &Attempt,
    ) -> Result<Reservation, BudgetError>;
    /// Exact settlement is idempotent, including after seal or expiry.
    async fn settle(
        &self,
        registration: &Registration,
        attempt_id: Uuid,
        settlement: &Settlement,
    ) -> Result<(), BudgetError>;
    async fn seal(&self, registration: &Registration) -> Result<(), BudgetError>;
}
