//! `/api/v1/agent-roles` -- Agent roles: the cost each role consumed over its
//! period, and its cap.
//!
//! Reading needs the `stats_read` scope for an API key; setting the operator
//! cap is admin only. A role appears once one of its agent steps ran.

pub mod get;
pub mod list;
pub mod update_cost_cap;

use ironflow_core::agent_role::{MAX_ROLE_NAME_LEN, is_valid_role_name};

use crate::error::ApiError;

/// Reject a role name a role could never have, before any lookup.
fn validate_role_name(name: &str) -> Result<(), ApiError> {
    if is_valid_role_name(name) {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "invalid agent role name '{name}': not blank, no leading or trailing whitespace, at most {MAX_ROLE_NAME_LEN} bytes"
        )))
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use chrono::Utc;
    use http_body_util::BodyExt;
    use ironflow_core::agent_role::CostPeriod;
    use ironflow_store::entities::{RoleCostReservationRequest, RoleCostSettlement};
    use rust_decimal::Decimal;
    use serde_json::{Value, from_slice, to_vec};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::routes::{RouterConfig, create_router};
    use crate::state::AppState;

    /// Send one request through the full router.
    pub(crate) async fn call(
        state: &AppState,
        method: &str,
        uri: &str,
        auth: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", auth);
        let body = match body {
            Some(body) => {
                request = request.header("content-type", "application/json");
                Body::from(to_vec(&body).expect("serialize body"))
            }
            None => Body::empty(),
        };
        // The limiter keys on the peer address, which `oneshot` does not provide.
        let config = RouterConfig {
            rate_limit_auth: None,
            rate_limit_general: None,
            ..RouterConfig::default()
        };
        let response = create_router(state.clone(), config)
            .oneshot(request.body(body).expect("build request"))
            .await
            .expect("call router");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("read body")
            .to_bytes();
        let json = if bytes.is_empty() {
            Value::Null
        } else {
            from_slice(&bytes).unwrap_or(Value::Null)
        };
        (status, json)
    }

    /// `GET uri` through the full router.
    pub(crate) async fn get(state: &AppState, uri: &str, auth: &str) -> (StatusCode, Value) {
        call(state, "GET", uri, auth, None).await
    }

    /// Record a settled step of `cost` for `role`, capped by `cap` in code,
    /// and hold `reserved` for a running one.
    pub(crate) async fn seed_role(
        state: &AppState,
        role: &str,
        cap: Option<Decimal>,
        cost: Decimal,
        reserved: Decimal,
    ) {
        let period_start = CostPeriod::Month.start(Utc::now());
        let settled = Uuid::now_v7();
        for (id, budget) in [(settled, cost), (Uuid::now_v7(), reserved)] {
            state
                .store
                .reserve_agent_role_cost(RoleCostReservationRequest {
                    reservation_id: id,
                    role: role.to_string(),
                    code_cap_usd: cap,
                    period: "month".to_string(),
                    period_start,
                    budget_usd: Some(budget),
                    ttl_secs: 600,
                })
                .await
                .expect("reserve");
            if id == settled {
                state
                    .store
                    .settle_agent_role_cost(RoleCostSettlement {
                        reservation_id: id,
                        period: "month".to_string(),
                        period_start,
                        cost_usd: cost,
                    })
                    .await
                    .expect("settle");
            }
        }
    }
}
