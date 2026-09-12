use std::sync::Arc;
use uuid::Uuid;

use crate::domain::{
    entities::diagnostic_budget::{
        BudgetError, BudgetSnapshot, Dispatch, Registration, Reservation, Settlement,
    },
    repositories::diagnostic_budget::DiagnosticBudgetRepository,
};

pub struct DiagnosticBudgetHandler {
    registration: Registration,
    repository: Arc<dyn DiagnosticBudgetRepository>,
}

impl DiagnosticBudgetHandler {
    pub fn new(
        registration: Registration,
        repository: Arc<dyn DiagnosticBudgetRepository>,
    ) -> Result<Self, BudgetError> {
        registration.validate()?;
        Ok(Self {
            registration,
            repository,
        })
    }

    pub async fn provision_once(&self) -> Result<(), BudgetError> {
        self.repository.provision(&self.registration).await
    }

    /// An expired/sealed registration is valid retained state, not a refill opportunity.
    pub async fn verify_startup(&self) -> Result<BudgetSnapshot, BudgetError> {
        self.repository.read(&self.registration).await
    }

    pub fn registration(&self) -> &Registration {
        &self.registration
    }

    fn check_scope(&self, budget_id: Uuid) -> Result<(), BudgetError> {
        if budget_id != self.registration.budget_id {
            return Err(BudgetError::NotFound);
        }
        Ok(())
    }

    pub async fn snapshot(&self, budget_id: Uuid) -> Result<BudgetSnapshot, BudgetError> {
        self.check_scope(budget_id)?;
        self.repository.read(&self.registration).await
    }

    pub async fn reserve(
        &self,
        budget_id: Uuid,
        dispatch: &Dispatch,
    ) -> Result<Reservation, BudgetError> {
        self.check_scope(budget_id)?;
        dispatch.validate()?;
        self.repository
            .reserve(&self.registration, &dispatch.attempt)
            .await
    }

    pub async fn settle(
        &self,
        budget_id: Uuid,
        attempt_id: Uuid,
        usage: &Settlement,
    ) -> Result<(), BudgetError> {
        self.check_scope(budget_id)?;
        self.repository
            .settle(&self.registration, attempt_id, usage)
            .await
    }

    pub async fn seal(&self, budget_id: Uuid) -> Result<(), BudgetError> {
        self.check_scope(budget_id)?;
        self.repository.seal(&self.registration).await
    }
}
