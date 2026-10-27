//! `PUT /api/v1/agent-roles/{name}/cost-cap` -- Set or clear the operator cap
//! of an agent role (admin only).

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;
use rust_decimal::Decimal;
use serde_json::json;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::{ApiKeyScope, EventKind, NewAuditLogEntry};

use super::validate_role_name;
use crate::authorization::AuthorizeExt;
use crate::entities::{AgentRoleResponse, UpdateAgentRoleCostCapRequest};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Set (`operator_cap_usd` a number) or clear (`null`) the operator cap of an
/// agent role. Admin only; audited.
///
/// The cap enforced is the lower of the cap declared in code and the operator
/// cap: an operator can lower the cap of a role, never raise it.
///
/// # Errors
///
/// - 400 if the name is not a valid role name or the cap is negative
/// - 401 if not authenticated
/// - 403 if the caller is not an admin
/// - 404 if no step of the role ever ran
/// - 422 if the body lacks `operator_cap_usd` or has unknown fields
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        put,
        path = "/api/v1/agent-roles/{name}/cost-cap",
        tags = ["agent-roles"],
        params(("name" = String, Path, description = "Agent role name")),
        request_body(content = UpdateAgentRoleCostCapRequest, description = "Operator cap, null to clear it"),
        responses(
            (status = 200, description = "Agent role updated", body = AgentRoleResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Agent role not found"),
            (status = 422, description = "Body without operator_cap_usd or with unknown fields")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn update_agent_role_cost_cap(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(name): Path<String>,
    Json(req): Json<UpdateAgentRoleCostCapRequest>,
) -> Result<impl IntoResponse, ApiError> {
    auth.require(ApiKeyScope::Admin, true)?;
    validate_role_name(&name)?;
    if req.operator_cap_usd.is_some_and(|cap| cap < Decimal::ZERO) {
        return Err(ApiError::BadRequest(
            "operator_cap_usd must not be negative".to_string(),
        ));
    }

    let record = state
        .store
        .set_agent_role_operator_cap(&name, req.operator_cap_usd)
        .await?;
    state
        .store
        .append_audit_log(NewAuditLogEntry {
            event_type: EventKind::AgentRoleCostCapUpdated,
            payload: json!({
                "role": name,
                "operator_cap_usd": req.operator_cap_usd,
            }),
            run_id: None,
            step_id: None,
            user_id: Some(auth.user_id),
        })
        .await?;
    Ok(ok(AgentRoleResponse::from_record(record, Utc::now())?))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use rust_decimal::Decimal;
    use serde_json::json;

    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_store::entities::EventKind;
    use ironflow_store::memory::InMemoryStore;

    use crate::routes::agent_roles::test_support::{call, seed_role};
    use crate::routes::test_helpers::{audit_entries, create_user_auth_header, state_with_engine};

    const URI: &str = "/api/v1/agent-roles/reviewer/cost-cap";

    #[tokio::test]
    async fn admin_lowers_the_cap_and_it_is_audited() {
        let store = Arc::new(InMemoryStore::new());
        let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
        let state = state_with_engine(store.clone(), engine, false);
        seed_role(
            &state,
            "reviewer",
            Some(Decimal::TEN),
            Decimal::TWO,
            Decimal::ZERO,
        )
        .await;
        let admin = create_user_auth_header(&state, "admin", true).await;

        let body = json!({"operator_cap_usd": 5.0});
        let (status, resp) = call(&state, "PUT", URI, &admin, Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(resp["data"]["cap_usd"], 5.0);
        assert_eq!(resp["data"]["remaining_usd"], 3.0);

        let entries = audit_entries(&store, EventKind::AgentRoleCostCapUpdated, None).await;
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].payload["role"], "reviewer");

        // Above the code cap: the code cap still applies.
        let body = json!({"operator_cap_usd": 50.0});
        let (_, resp) = call(&state, "PUT", URI, &admin, Some(body)).await;
        assert_eq!(resp["data"]["cap_usd"], 10.0);

        let body = json!({"operator_cap_usd": null});
        let (status, resp) = call(&state, "PUT", URI, &admin, Some(body)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(resp["data"]["operator_cap_usd"].is_null());
    }

    #[tokio::test]
    async fn invalid_input_and_non_admins_are_refused() {
        let store = Arc::new(InMemoryStore::new());
        let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
        let state = state_with_engine(store, engine, false);
        seed_role(
            &state,
            "reviewer",
            Some(Decimal::TEN),
            Decimal::ONE,
            Decimal::ZERO,
        )
        .await;
        let admin = create_user_auth_header(&state, "admin", true).await;
        let member = create_user_auth_header(&state, "member", false).await;

        let body = json!({"operator_cap_usd": 1.0});
        let (status, _) = call(&state, "PUT", URI, &member, Some(body.clone())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let negative = json!({"operator_cap_usd": -1.0});
        let (status, _) = call(&state, "PUT", URI, &admin, Some(negative)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let unknown = "/api/v1/agent-roles/ghost/cost-cap";
        let (status, _) = call(&state, "PUT", unknown, &admin, Some(body.clone())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let invalid = "/api/v1/agent-roles/Not%20Valid/cost-cap";
        let (status, _) = call(&state, "PUT", invalid, &admin, Some(body)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
