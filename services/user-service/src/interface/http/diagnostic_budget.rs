use std::{sync::Arc, time::Duration};

use axum::{
    body::to_bytes,
    extract::{Path, Request, State},
    http::{
        header::{CACHE_CONTROL, CONTENT_TYPE},
        StatusCode,
    },
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use chrono::SecondsFormat;
use llm_client::diagnostic_budget::{
    canonical_uuid, Amount, Binding, ReservationResponse, ReserveRequest, SealRequest,
    SealResponse, SettleRequest, SettlementResponse, SnapshotResponse, CONTRACT_HEADER,
    MAX_CONTROL_BYTES,
};
use serde::{de::DeserializeOwned, Serialize};

use crate::{
    application::diagnostic_budget::DiagnosticBudgetHandler,
    domain::entities::diagnostic_budget::{Attempt, BudgetError, Dispatch, Profile, Settlement},
};

#[derive(Clone)]
struct BudgetHttpState {
    handler: Option<Arc<DiagnosticBudgetHandler>>,
    token: Arc<str>,
}

pub fn router(handler: Option<Arc<DiagnosticBudgetHandler>>, token: Arc<str>) -> Router {
    Router::new()
        .route("/internal/llm-budget/{budget_id}", get(snapshot))
        .route("/internal/llm-budget/{budget_id}/reserve", post(reserve))
        .route("/internal/llm-budget/{budget_id}/settle", post(settle))
        .route("/internal/llm-budget/{budget_id}/seal", post(seal))
        .with_state(BudgetHttpState { handler, token })
}

pub(super) fn binding(handler: &DiagnosticBudgetHandler) -> Binding {
    let registration = handler.registration();
    Binding {
        contract: registration.contract.clone(),
        profile: registration.profile.clone(),
        profile_sha256: registration.profile_sha256.clone(),
        budget_id: registration.budget_id,
    }
}

struct ControlFailure(StatusCode, &'static str);

impl From<BudgetError> for ControlFailure {
    fn from(error: BudgetError) -> Self {
        match error {
            BudgetError::Invalid => Self(StatusCode::BAD_REQUEST, "diagnostic_budget_invalid"),
            BudgetError::NotFound => Self(StatusCode::NOT_FOUND, "diagnostic_budget_not_found"),
            BudgetError::Conflict => Self(StatusCode::CONFLICT, "diagnostic_budget_conflict"),
            BudgetError::Exhausted => {
                Self(StatusCode::TOO_MANY_REQUESTS, "diagnostic_budget_exhausted")
            }
            BudgetError::Closed => Self(StatusCode::GONE, "diagnostic_budget_closed"),
            BudgetError::Unavailable => Self(
                StatusCode::SERVICE_UNAVAILABLE,
                "diagnostic_budget_unavailable",
            ),
        }
    }
}

fn authorize(
    state: &BudgetHttpState,
    id: &str,
    request: &Request,
) -> Result<Arc<DiagnosticBudgetHandler>, ControlFailure> {
    if request
        .headers()
        .get_all("X-Internal-Service-Token")
        .iter()
        .count()
        != 1
        || !super::internal_token_authorized(request.headers(), &state.token)
    {
        return Err(ControlFailure(
            StatusCode::UNAUTHORIZED,
            "diagnostic_budget_unauthorized",
        ));
    }
    if request.uri().query().is_some()
        || request.headers().get_all(CONTRACT_HEADER).iter().count() != 1
        || request
            .headers()
            .get(CONTRACT_HEADER)
            .and_then(|value| value.to_str().ok())
            != Some(Profile::compiled().contract.as_str())
    {
        return Err(BudgetError::Invalid.into());
    }
    let id = canonical_uuid(id).map_err(|_| BudgetError::Invalid)?;
    let handler = state.handler.as_ref().ok_or(BudgetError::NotFound)?;
    if id != handler.registration().budget_id {
        return Err(BudgetError::NotFound.into());
    }
    Ok(handler.clone())
}

fn verify_binding(
    handler: &DiagnosticBudgetHandler,
    actual: &Binding,
) -> Result<(), ControlFailure> {
    if actual.budget_id != handler.registration().budget_id {
        return Err(BudgetError::NotFound.into());
    }
    if *actual != binding(handler) {
        return Err(BudgetError::Invalid.into());
    }
    Ok(())
}

async fn decode<T: DeserializeOwned>(request: Request) -> Result<T, ControlFailure> {
    if !request
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(BudgetError::Invalid.into());
    }
    let body = tokio::time::timeout(
        Duration::from_secs(5),
        to_bytes(request.into_body(), MAX_CONTROL_BYTES),
    )
    .await
    .map_err(|_| BudgetError::Unavailable)?
    .map_err(|_| BudgetError::Invalid)?;
    serde_json::from_slice(&body).map_err(|_| BudgetError::Invalid.into())
}

fn amount(amount: crate::domain::entities::diagnostic_budget::Amount) -> Amount {
    Amount {
        attempts: amount.attempts,
        tokens: amount.tokens,
        cost_micro_cny: amount.cost_micro_cny,
    }
}

fn respond<T: Serialize>(result: Result<T, ControlFailure>) -> Response {
    let result = result.and_then(|value| {
        let bytes = serde_json::to_vec(&value).map_err(|_| BudgetError::Unavailable)?;
        if bytes.len() > MAX_CONTROL_BYTES {
            return Err(BudgetError::Unavailable.into());
        }
        Ok(bytes)
    });
    let (status, body) = match result {
        Ok(body) => (StatusCode::OK, body),
        Err(ControlFailure(status, code)) => (
            status,
            serde_json::to_vec(&serde_json::json!({
                "contract": Profile::compiled().contract, "code": code,
            }))
            .expect("constant control error serializes"),
        ),
    };
    (
        status,
        [
            (CONTENT_TYPE, "application/json"),
            (CACHE_CONTROL, "no-store"),
        ],
        body,
    )
        .into_response()
}

async fn snapshot(
    State(state): State<BudgetHttpState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    respond(
        async {
            let handler = authorize(&state, &id, &request)?;
            tokio::time::timeout(Duration::from_secs(5), to_bytes(request.into_body(), 0))
                .await
                .map_err(|_| BudgetError::Unavailable)?
                .map_err(|_| BudgetError::Invalid)?;
            let snapshot = handler.snapshot(handler.registration().budget_id).await?;
            Ok(SnapshotResponse {
                binding: binding(&handler),
                limits: amount(snapshot.registration.limits),
                charged: amount(snapshot.charged),
                expires_at: snapshot
                    .registration
                    .expires_at
                    .to_rfc3339_opts(SecondsFormat::Secs, true),
                sealed: snapshot.sealed,
            })
        }
        .await,
    )
}

async fn reserve(
    State(state): State<BudgetHttpState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    respond(
        async {
            let handler = authorize(&state, &id, &request)?;
            let request: ReserveRequest = decode(request).await?;
            verify_binding(&handler, &request.binding)?;
            let receipt = handler
                .reserve(
                    request.binding.budget_id,
                    &Dispatch {
                        attempt: Attempt {
                            attempt_id: request.attempt_id,
                            operation: request.operation,
                            output_limit: request.output_limit,
                        },
                        provider: request.provider,
                        model: request.model,
                        origin: request.origin,
                    },
                )
                .await?;
            Ok(ReservationResponse {
                binding: binding(&handler),
                attempt_id: receipt.attempt_id,
                ordinal: receipt.ordinal,
                reservation: amount(receipt.amount),
            })
        }
        .await,
    )
}

async fn settle(
    State(state): State<BudgetHttpState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    respond(
        async {
            let handler = authorize(&state, &id, &request)?;
            let request: SettleRequest = decode(request).await?;
            verify_binding(&handler, &request.binding)?;
            handler
                .settle(
                    request.binding.budget_id,
                    request.attempt_id,
                    &Settlement {
                        model: request.usage.model.clone(),
                        input_tokens: request.usage.input_tokens,
                        output_tokens: request.usage.output_tokens,
                        cached_input_tokens: request.usage.cached_input_tokens,
                    },
                )
                .await?;
            Ok(SettlementResponse {
                binding: binding(&handler),
                attempt_id: request.attempt_id,
                usage: request.usage,
            })
        }
        .await,
    )
}

async fn seal(
    State(state): State<BudgetHttpState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    respond(
        async {
            let handler = authorize(&state, &id, &request)?;
            let request: SealRequest = decode(request).await?;
            verify_binding(&handler, &request.binding)?;
            handler.seal(request.binding.budget_id).await?;
            Ok(SealResponse {
                binding: binding(&handler),
                sealed: true,
            })
        }
        .await,
    )
}
