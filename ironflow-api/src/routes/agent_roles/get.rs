//! `GET /api/v1/agent-roles/{name}` -- Cost and cap of one agent role.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use chrono::Utc;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::ApiKeyScope;

use super::validate_role_name;
use crate::authorization::AuthorizeExt;
use crate::entities::AgentRoleResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Get the cost spent and reserved by an agent role over its current period,
/// its cap and the room left.
///
/// An API key needs the `stats_read` scope.
///
/// # Errors
///
/// - 400 if the name is not a valid role name
/// - 401 if not authenticated
/// - 403 if the API key lacks `stats_read`
/// - 404 if no step of the role ever ran
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/agent-roles/{name}",
        tags = ["agent-roles"],
        params(("name" = String, Path, description = "Agent role name")),
        responses(
            (status = 200, description = "Agent role", body = AgentRoleResponse),
            (status = 400, description = "Invalid role name"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Insufficient scope"),
            (status = 404, description = "Agent role not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_agent_role(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    auth.require(ApiKeyScope::StatsRead, false)?;
    validate_role_name(&name)?;
    let record = state
        .store
        .get_agent_role(&name)
        .await?
        .ok_or(ApiError::AgentRoleNotFound(name))?;
    Ok(ok(AgentRoleResponse::from_record(record, Utc::now())?))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use rust_decimal::Decimal;

    use crate::routes::agent_roles::test_support::{get, seed_role};
    use crate::routes::test_helpers::{create_user_auth_header, test_state};

    #[tokio::test]
    async fn shows_spent_reserved_and_remaining() {
        let state = test_state();
        seed_role(
            &state,
            "reviewer",
            Some(Decimal::TEN),
            Decimal::TWO,
            Decimal::ONE,
        )
        .await;
        let auth = create_user_auth_header(&state, "member", false).await;

        let (status, body) = get(&state, "/api/v1/agent-roles/reviewer", &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["name"], "reviewer");
        assert_eq!(body["data"]["spent_usd"], 2.0);
        assert_eq!(body["data"]["reserved_usd"], 1.0);
        assert_eq!(body["data"]["remaining_usd"], 7.0);
        assert_eq!(body["data"]["period"], "month");
    }

    #[tokio::test]
    async fn unknown_and_invalid_names_are_rejected() {
        let state = test_state();
        let auth = create_user_auth_header(&state, "member", false).await;

        let (status, body) = get(&state, "/api/v1/agent-roles/ghost", &auth).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"]["code"], "AGENT_ROLE_NOT_FOUND");

        let (status, _) = get(&state, "/api/v1/agent-roles/Ghost", &auth).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
