//! Worker-only agent role routes (WORKER_TOKEN auth).
//!
//! - `POST /api/v1/internal/agent-roles/reservations` -- reserve room for an
//!   agent step of a role, or refuse it at the cap.
//! - `PUT /api/v1/internal/agent-roles/reservations/{id}/lease` -- extend a
//!   reservation while its step runs.
//! - `PUT /api/v1/internal/agent-roles/settlements` -- release a reservation
//!   and add the cost of its step.

use std::str::FromStr;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use rust_decimal::Decimal;
use serde::Deserialize;
use uuid::Uuid;

use ironflow_core::agent_role::{CostPeriod, is_valid_role_name};
use ironflow_store::entities::{RoleCostReservationRequest, RoleCostSettlement};

use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Longest reservation lease a worker may ask for, in seconds.
const MAX_TTL_SECS: u32 = 3600;

/// Body of the lease renewal route.
#[derive(Debug, Deserialize)]
pub struct RenewReservationBody {
    /// New lifetime of the reservation, in seconds from now.
    pub ttl_secs: u32,
}

fn validate_ttl(ttl_secs: u32) -> Result<(), ApiError> {
    if (1..=MAX_TTL_SECS).contains(&ttl_secs) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "ttl_secs must be between 1 and {MAX_TTL_SECS}, got {ttl_secs}"
        )))
    }
}

fn validate_period(period: &str) -> Result<(), ApiError> {
    CostPeriod::from_str(period)
        .map(|_| ())
        .map_err(|e| ApiError::BadRequest(format!("invalid period '{period}': {e}")))
}

fn validate_non_negative(field: &str, value: Option<Decimal>) -> Result<(), ApiError> {
    match value {
        Some(v) if v < Decimal::ZERO => Err(ApiError::BadRequest(format!(
            "{field} must not be negative"
        ))),
        _ => Ok(()),
    }
}

/// Reserve room for one agent step of a role. Worker-only.
pub async fn reserve_agent_role_cost(
    State(state): State<AppState>,
    Json(request): Json<RoleCostReservationRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !is_valid_role_name(&request.role) {
        let role = &request.role;
        return Err(ApiError::BadRequest(format!(
            "invalid agent role name '{role}'"
        )));
    }
    validate_period(&request.period)?;
    validate_ttl(request.ttl_secs)?;
    validate_non_negative("code_cap_usd", request.code_cap_usd)?;
    validate_non_negative("budget_usd", request.budget_usd)?;
    let admission = state.store.reserve_agent_role_cost(request).await?;
    Ok(ok(admission))
}

/// Extend a reservation. Worker-only; an unknown reservation is a no-op.
pub async fn renew_agent_role_reservation(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<RenewReservationBody>,
) -> Result<impl IntoResponse, ApiError> {
    validate_ttl(body.ttl_secs)?;
    state
        .store
        .renew_agent_role_reservation(id, body.ttl_secs)
        .await?;
    Ok(ok(()))
}

/// Settle a reservation with the cost of its step. Worker-only; idempotent.
pub async fn settle_agent_role_cost(
    State(state): State<AppState>,
    Json(settlement): Json<RoleCostSettlement>,
) -> Result<impl IntoResponse, ApiError> {
    validate_period(&settlement.period)?;
    validate_non_negative("cost_usd", Some(settlement.cost_usd))?;
    state.store.settle_agent_role_cost(settlement).await?;
    Ok(ok(()))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use chrono::Utc;
    use serde_json::{Value, json};

    use crate::routes::agent_roles::test_support::call;
    use crate::routes::test_helpers::test_state;

    use super::*;

    const WORKER: &str = "Bearer test-worker-token";

    fn reservation(id: Uuid, role: &str) -> Value {
        json!({
            "reservation_id": id,
            "role": role,
            "code_cap_usd": 1.0,
            "period": "month",
            "period_start": CostPeriod::Month.start(Utc::now()),
            "budget_usd": 0.6,
            "ttl_secs": 600,
        })
    }

    #[tokio::test]
    async fn reserve_renew_and_settle_round_trip() {
        let state = test_state();
        let id = Uuid::now_v7();
        let uri = "/api/v1/internal/agent-roles/reservations";
        let body = reservation(id, "reviewer");
        let (status, body) = call(&state, "POST", uri, WORKER, Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["outcome"], "admitted");

        let lease = format!("/api/v1/internal/agent-roles/reservations/{id}/lease");
        let (status, _) = call(
            &state,
            "PUT",
            &lease,
            WORKER,
            Some(json!({"ttl_secs": 600})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let settlement = json!({
            "reservation_id": id,
            "period": "month",
            "period_start": CostPeriod::Month.start(Utc::now()),
            "cost_usd": 0.6,
        });
        let uri = "/api/v1/internal/agent-roles/settlements";
        let (status, _) = call(&state, "PUT", uri, WORKER, Some(settlement)).await;
        assert_eq!(status, StatusCode::OK);

        let record = state
            .store
            .get_agent_role("reviewer")
            .await
            .unwrap()
            .expect("role recorded");
        assert_eq!(record.spent_usd, Decimal::new(6, 1));
        assert_eq!(record.reserved_usd, Decimal::ZERO);
    }

    #[tokio::test]
    async fn invalid_reservations_are_rejected() {
        let state = test_state();
        let uri = "/api/v1/internal/agent-roles/reservations";
        let bad_name = reservation(Uuid::now_v7(), "Not Valid");
        let (status, _) = call(&state, "POST", uri, WORKER, Some(bad_name)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let mut bad_ttl = reservation(Uuid::now_v7(), "reviewer");
        bad_ttl["ttl_secs"] = json!(0);
        let (status, _) = call(&state, "POST", uri, WORKER, Some(bad_ttl)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let mut bad_period = reservation(Uuid::now_v7(), "reviewer");
        bad_period["period"] = json!("week");
        let (status, _) = call(&state, "POST", uri, WORKER, Some(bad_period)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let mut negative = reservation(Uuid::now_v7(), "reviewer");
        negative["budget_usd"] = json!(-1.0);
        let (status, _) = call(&state, "POST", uri, WORKER, Some(negative)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn internal_agent_role_routes_require_the_worker_token() {
        let state = test_state();
        let uri = "/api/v1/internal/agent-roles/reservations";
        let body = reservation(Uuid::now_v7(), "reviewer");
        let (status, _) = call(&state, "POST", uri, "Bearer wrong", Some(body)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
