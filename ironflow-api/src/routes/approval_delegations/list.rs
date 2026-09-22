//! `GET /api/v1/approval-delegations` -- List active approval delegations.

use std::cmp::Reverse;

use axum::extract::{Query, State};
use axum::response::IntoResponse;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::DelegationFilter;

use crate::entities::{ApprovalDelegationResponse, ListApprovalDelegationsQuery};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// List the active approval delegations visible to the caller.
///
/// An admin sees every active delegation and may narrow the result with the
/// query parameters. A non-admin always sees exactly the delegations they
/// granted plus the ones they received, and the query parameters are ignored --
/// they must not become a way to enumerate other people's delegations.
///
/// Expired and not-yet-started delegations are never returned: the store filters
/// them out at read time.
///
/// # Errors
///
/// - 401 if not authenticated
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/approval-delegations",
        tags = ["approval-delegations"],
        params(ListApprovalDelegationsQuery),
        responses(
            (status = 200, description = "Active delegations", body = Vec<ApprovalDelegationResponse>),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_approval_delegations(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<ListApprovalDelegationsQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let delegations = if auth.is_admin() {
        state
            .store
            .list_active_delegations(DelegationFilter {
                from_user_id: query.from_user_id,
                to_user_id: query.to_user_id,
            })
            .await?
    } else {
        let mut granted = state
            .store
            .list_active_delegations(DelegationFilter {
                from_user_id: Some(auth.user_id),
                ..DelegationFilter::default()
            })
            .await?;
        let received = state
            .store
            .list_active_delegations(DelegationFilter {
                to_user_id: Some(auth.user_id),
                ..DelegationFilter::default()
            })
            .await?;
        // The two sets are disjoint: self-delegation is refused at creation, so
        // no row can have the caller on both sides. No dedup needed.
        granted.extend(received);
        granted.sort_by_key(|d| Reverse(d.created_at));
        granted
    };

    let items: Vec<ApprovalDelegationResponse> = delegations
        .into_iter()
        .map(ApprovalDelegationResponse::from)
        .collect();

    Ok(ok(items))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use chrono::{TimeDelta, Utc};
    use http_body_util::BodyExt;
    use ironflow_store::entities::NewApprovalDelegation;
    use serde_json::{Value, from_slice};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::routes::approval_delegations::test_support::{
        Users, admin_header, member_header, test_state,
    };

    use super::*;

    /// An active, unfiltered delegation.
    fn active(from: Uuid, to: Uuid) -> NewApprovalDelegation {
        let now = Utc::now();
        NewApprovalDelegation {
            from_user_id: from,
            to_user_id: to,
            valid_from: now - TimeDelta::hours(1),
            valid_until: now + TimeDelta::hours(1),
            workflow_filter: None,
        }
    }

    /// `GET uri`, returning the IDs of the delegations in the response body.
    async fn list_ids(state: AppState, auth: &str, uri: &str) -> Vec<String> {
        let app = Router::new()
            .route("/", get(list_approval_delegations))
            .with_state(state);

        let req = Request::builder()
            .uri(uri)
            .header("authorization", auth)
            .body(Body::empty())
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::OK);

        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let value: Value = from_slice(&bytes).expect("json");
        value["data"]
            .as_array()
            .expect("data array")
            .iter()
            .map(|d| d["id"].as_str().expect("id").to_string())
            .collect()
    }

    /// alice -> bob, carol -> alice, bob -> carol, plus an expired alice -> carol.
    async fn seed(state: &AppState, users: &Users) -> (String, String, String, String) {
        let granted_by_alice = state
            .store
            .create_delegation(active(users.alice.id, users.bob.id))
            .await
            .expect("alice -> bob");
        let received_by_alice = state
            .store
            .create_delegation(active(users.carol.id, users.alice.id))
            .await
            .expect("carol -> alice");
        let third_party = state
            .store
            .create_delegation(active(users.bob.id, users.carol.id))
            .await
            .expect("bob -> carol");

        let now = Utc::now();
        let expired = state
            .store
            .create_delegation(NewApprovalDelegation {
                valid_from: now - TimeDelta::days(10),
                valid_until: now - TimeDelta::days(3),
                ..active(users.alice.id, users.carol.id)
            })
            .await
            .expect("expired alice -> carol");

        (
            granted_by_alice.id.to_string(),
            received_by_alice.id.to_string(),
            third_party.id.to_string(),
            expired.id.to_string(),
        )
    }

    #[tokio::test]
    async fn a_member_sees_only_their_own_delegations() {
        let (state, users) = test_state().await;
        let (granted, received, third_party, expired) = seed(&state, &users).await;
        let auth = member_header(&users.alice, &state);

        let ids = list_ids(state, &auth, "/").await;

        assert!(ids.contains(&granted), "the delegation alice granted");
        assert!(ids.contains(&received), "the delegation alice received");
        assert!(!ids.contains(&third_party), "someone else's delegation");
        assert!(!ids.contains(&expired), "an expired delegation");
        assert_eq!(ids.len(), 2);
    }

    #[tokio::test]
    async fn a_member_cannot_widen_the_result_with_query_parameters() {
        let (state, users) = test_state().await;
        let (_, _, third_party, _) = seed(&state, &users).await;
        let auth = member_header(&users.alice, &state);

        let uri = format!("/?to_user_id={}", users.carol.id);
        let ids = list_ids(state, &auth, &uri).await;

        assert!(!ids.contains(&third_party));
        assert_eq!(ids.len(), 2);
    }

    #[tokio::test]
    async fn an_admin_sees_every_active_delegation() {
        let (state, users) = test_state().await;
        let (granted, received, third_party, expired) = seed(&state, &users).await;
        let auth = admin_header(&users.alice, &state);

        let ids = list_ids(state, &auth, "/").await;

        assert!(ids.contains(&granted));
        assert!(ids.contains(&received));
        assert!(ids.contains(&third_party));
        assert!(!ids.contains(&expired), "an expired delegation");
        assert_eq!(ids.len(), 3);
    }

    #[tokio::test]
    async fn an_admin_can_filter_by_delegate() {
        let (state, users) = test_state().await;
        let (granted, _, third_party, _) = seed(&state, &users).await;
        let auth = admin_header(&users.alice, &state);

        let uri = format!("/?to_user_id={}", users.bob.id);
        let ids = list_ids(state, &auth, &uri).await;

        assert_eq!(ids, vec![granted]);
        assert!(!ids.contains(&third_party));
    }
}
