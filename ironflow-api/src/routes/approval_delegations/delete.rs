//! `DELETE /api/v1/approval-delegations/{id}` -- Revoke an approval delegation.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;

use crate::error::ApiError;
use crate::state::AppState;

/// Revoke an approval delegation.
///
/// Only the delegator who granted it, or an admin, may revoke a delegation.
/// Expired rows are still revocable: they are readable by ID even though they
/// no longer appear in the list.
///
/// # Errors
///
/// - 401 if not authenticated
/// - 403 if the caller is neither the delegator nor an admin
/// - 404 if the delegation does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        delete,
        path = "/api/v1/approval-delegations/{id}",
        tags = ["approval-delegations"],
        params(("id" = Uuid, Path, description = "Delegation ID")),
        responses(
            (status = 204, description = "Delegation revoked"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden"),
            (status = 404, description = "Delegation not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn delete_approval_delegation(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    let delegation = state
        .store
        .find_delegation_by_id(id)
        .await?
        .ok_or(ApiError::DelegationNotFound(id))?;

    if !auth.is_admin() && delegation.from_user_id != auth.user_id {
        return Err(ApiError::Forbidden);
    }

    state.store.delete_delegation(id).await?;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::delete;
    use chrono::{TimeDelta, Utc};
    use ironflow_store::entities::{ApprovalDelegation, DelegationFilter, NewApprovalDelegation};
    use tower::ServiceExt;

    use crate::routes::approval_delegations::test_support::{
        Users, admin_header, member_header, test_state,
    };

    use super::*;

    /// An active delegation from alice to bob.
    async fn alice_to_bob(state: &AppState, users: &Users) -> ApprovalDelegation {
        let now = Utc::now();
        state
            .store
            .create_delegation(NewApprovalDelegation {
                from_user_id: users.alice.id,
                to_user_id: users.bob.id,
                valid_from: now - TimeDelta::hours(1),
                valid_until: now + TimeDelta::hours(1),
                workflow_filter: None,
            })
            .await
            .expect("create delegation")
    }

    async fn revoke(state: AppState, auth: &str, id: Uuid) -> StatusCode {
        let app = Router::new()
            .route("/{id}", delete(delete_approval_delegation))
            .with_state(state);

        let req = Request::builder()
            .uri(format!("/{id}"))
            .method("DELETE")
            .header("authorization", auth)
            .body(Body::empty())
            .expect("build");

        app.oneshot(req).await.expect("request").status()
    }

    #[tokio::test]
    async fn the_delegator_can_revoke_and_the_row_disappears() {
        let (state, users) = test_state().await;
        let delegation = alice_to_bob(&state, &users).await;
        let auth = member_header(&users.alice, &state);
        let store = state.store.clone();

        assert_eq!(
            revoke(state, &auth, delegation.id).await,
            StatusCode::NO_CONTENT
        );

        let remaining = store
            .list_active_delegations(DelegationFilter::default(), 1, 100)
            .await
            .expect("list");
        assert_eq!(remaining.total, 0);
    }

    #[tokio::test]
    async fn a_third_party_member_cannot_revoke() {
        let (state, users) = test_state().await;
        let delegation = alice_to_bob(&state, &users).await;
        let auth = member_header(&users.carol, &state);
        let store = state.store.clone();

        assert_eq!(
            revoke(state, &auth, delegation.id).await,
            StatusCode::FORBIDDEN
        );

        assert!(
            store
                .find_delegation_by_id(delegation.id)
                .await
                .expect("find")
                .is_some(),
            "a refused revocation must leave the delegation in place"
        );
    }

    #[tokio::test]
    async fn the_delegate_cannot_revoke_a_delegation_they_received() {
        let (state, users) = test_state().await;
        let delegation = alice_to_bob(&state, &users).await;
        let auth = member_header(&users.bob, &state);

        assert_eq!(
            revoke(state, &auth, delegation.id).await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn an_admin_can_revoke_someone_elses_delegation() {
        let (state, users) = test_state().await;
        let delegation = alice_to_bob(&state, &users).await;
        let auth = admin_header(&users.carol, &state);

        assert_eq!(
            revoke(state, &auth, delegation.id).await,
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn an_unknown_id_is_not_found() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);

        assert_eq!(
            revoke(state, &auth, Uuid::now_v7()).await,
            StatusCode::NOT_FOUND
        );
    }
}
