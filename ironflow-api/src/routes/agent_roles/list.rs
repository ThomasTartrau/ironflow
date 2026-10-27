//! `GET /api/v1/agent-roles` -- List agent roles with their cost and cap.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use chrono::Utc;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::ApiKeyScope;

use crate::authorization::AuthorizeExt;
use crate::entities::{AgentRoleResponse, ListAgentRolesQuery};
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// List the agent roles seen by the platform, ordered by name, with the cost
/// spent and reserved over their current period and the room left.
///
/// An API key needs the `stats_read` scope.
///
/// # Errors
///
/// - 401 if not authenticated
/// - 403 if the API key lacks `stats_read`
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/agent-roles",
        tags = ["agent-roles"],
        params(ListAgentRolesQuery),
        responses(
            (status = 200, description = "Paginated list of agent roles", body = Vec<AgentRoleResponse>),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Insufficient scope")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_agent_roles(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<ListAgentRolesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    auth.require(ApiKeyScope::StatsRead, false)?;
    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).clamp(1, 100);

    let result = state.store.list_agent_roles(page, per_page).await?;
    let now = Utc::now();
    let items = result
        .items
        .into_iter()
        .map(|record| AgentRoleResponse::from_record(record, now))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(ok_paged(items, page, per_page, result.total))
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;
    use rust_decimal::Decimal;

    use crate::routes::agent_roles::test_support::{get, seed_role};
    use crate::routes::test_helpers::{create_user_auth_header, test_state};

    #[tokio::test]
    async fn lists_roles_by_name_with_pagination() {
        let state = test_state();
        seed_role(&state, "triage", None, Decimal::ONE, Decimal::ZERO).await;
        seed_role(
            &state,
            "reviewer",
            Some(Decimal::TEN),
            Decimal::TWO,
            Decimal::ONE,
        )
        .await;
        let auth = create_user_auth_header(&state, "member", false).await;

        let (status, body) = get(&state, "/api/v1/agent-roles?per_page=1", &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"][0]["name"], "reviewer");
        assert_eq!(body["meta"]["total"], 2);

        let second_page = "/api/v1/agent-roles?page=2&per_page=1";
        let (status, body) = get(&state, second_page, &auth).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"][0]["name"], "triage");
        assert!(body["data"][0]["cap_usd"].is_null());
    }

    #[tokio::test]
    async fn listing_requires_authentication() {
        let state = test_state();
        let (status, _) = get(&state, "/api/v1/agent-roles", "Bearer nope").await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
