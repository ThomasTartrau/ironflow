//! `POST /api/v1/approval-delegations` -- Delegate approval power to someone else.

use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use chrono::Utc;
use validator::Validate;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::entities::{NewApprovalDelegation, validate_workflow_filter};

use crate::entities::{ApprovalDelegationResponse, CreateApprovalDelegationRequest};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Create an approval delegation.
///
/// The delegator is always the caller: a user hands over their own approval
/// power, never someone else's. Any authenticated user may do so.
///
/// # Errors
///
/// - 400 if the target is the caller, the window is inverted, the workflow
///   filter is not a valid glob, or the target user does not exist
/// - 401 if not authenticated
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        post,
        path = "/api/v1/approval-delegations",
        tags = ["approval-delegations"],
        request_body(content = CreateApprovalDelegationRequest, description = "Delegation definition"),
        responses(
            (status = 201, description = "Delegation created", body = ApprovalDelegationResponse),
            (status = 400, description = "Invalid input"),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn create_approval_delegation(
    auth: Authenticated,
    State(state): State<AppState>,
    Json(req): Json<CreateApprovalDelegationRequest>,
) -> Result<impl IntoResponse, ApiError> {
    req.validate()
        .map_err(|e| ApiError::BadRequest(e.to_string()))?;

    if req.to_user_id == auth.user_id {
        return Err(ApiError::BadRequest(
            "cannot delegate approvals to yourself".to_string(),
        ));
    }

    let valid_from = req.valid_from.unwrap_or_else(Utc::now);
    if req.valid_until <= valid_from {
        return Err(ApiError::BadRequest(
            "valid_until must be after valid_from".to_string(),
        ));
    }

    if let Some(filter) = &req.workflow_filter {
        validate_workflow_filter(filter).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    }

    state
        .store
        .find_user_by_id(req.to_user_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("target user does not exist".to_string()))?;

    let delegation = state
        .store
        .create_delegation(NewApprovalDelegation {
            from_user_id: auth.user_id,
            to_user_id: req.to_user_id,
            valid_from,
            valid_until: req.valid_until,
            workflow_filter: req.workflow_filter,
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        ok(ApprovalDelegationResponse::from(delegation)),
    ))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::Request;
    use axum::routing::post;
    use chrono::TimeDelta;
    use http_body_util::BodyExt;
    use serde_json::{Value, from_slice, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::routes::approval_delegations::test_support::{member_header, test_state};

    use super::*;

    /// `POST /` with `body`, returning the status and the parsed JSON body.
    async fn post_delegation(
        state: AppState,
        auth: Option<&str>,
        body: Value,
    ) -> (StatusCode, Value) {
        let app = Router::new()
            .route("/", post(create_approval_delegation))
            .with_state(state);

        let mut builder = Request::builder()
            .uri("/")
            .method("POST")
            .header("content-type", "application/json");
        if let Some(auth) = auth {
            builder = builder.header("authorization", auth);
        }
        let req = builder.body(Body::from(body.to_string())).expect("build");

        let resp = app.oneshot(req).await.expect("request");
        let status = resp.status();
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let value = if bytes.is_empty() {
            Value::Null
        } else {
            from_slice(&bytes).expect("json")
        };
        (status, value)
    }

    #[tokio::test]
    async fn create_without_a_workflow_filter() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);
        let until = Utc::now() + TimeDelta::days(7);

        let (status, body) = post_delegation(
            state,
            Some(&auth),
            json!({ "to_user_id": users.bob.id, "valid_until": until }),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["data"]["from_user_id"], users.alice.id.to_string());
        assert_eq!(body["data"]["to_user_id"], users.bob.id.to_string());
        assert!(body["data"]["workflow_filter"].is_null());
        assert!(body["data"]["valid_from"].is_string());
    }

    #[tokio::test]
    async fn create_with_a_workflow_filter() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);
        let until = Utc::now() + TimeDelta::days(7);

        let (status, body) = post_delegation(
            state,
            Some(&auth),
            json!({
                "to_user_id": users.bob.id,
                "valid_until": until,
                "workflow_filter": "deploy-*",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(body["data"]["workflow_filter"], "deploy-*");
    }

    #[tokio::test]
    async fn self_delegation_is_refused() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);
        let until = Utc::now() + TimeDelta::days(1);

        let (status, body) = post_delegation(
            state,
            Some(&auth),
            json!({ "to_user_id": users.alice.id, "valid_until": until }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"]["message"]
                .as_str()
                .expect("message")
                .contains("yourself")
        );
    }

    #[tokio::test]
    async fn an_unknown_target_user_is_refused() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);
        let until = Utc::now() + TimeDelta::days(1);

        let (status, body) = post_delegation(
            state,
            Some(&auth),
            json!({ "to_user_id": Uuid::now_v7(), "valid_until": until }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"]["message"]
                .as_str()
                .expect("message")
                .contains("target user does not exist")
        );
    }

    #[tokio::test]
    async fn an_inverted_window_is_refused() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);
        let from = Utc::now();

        let (status, body) = post_delegation(
            state,
            Some(&auth),
            json!({
                "to_user_id": users.bob.id,
                "valid_from": from,
                "valid_until": from - TimeDelta::hours(1),
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"]["message"]
                .as_str()
                .expect("message")
                .contains("valid_until must be after valid_from")
        );
    }

    #[tokio::test]
    async fn an_empty_window_is_refused() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);
        let instant = Utc::now();

        let (status, _) = post_delegation(
            state,
            Some(&auth),
            json!({
                "to_user_id": users.bob.id,
                "valid_from": instant,
                "valid_until": instant,
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn an_empty_workflow_filter_is_refused() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);
        let until = Utc::now() + TimeDelta::days(1);

        let (status, _) = post_delegation(
            state,
            Some(&auth),
            json!({
                "to_user_id": users.bob.id,
                "valid_until": until,
                "workflow_filter": "",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_broken_glob_is_refused() {
        let (state, users) = test_state().await;
        let auth = member_header(&users.alice, &state);
        let until = Utc::now() + TimeDelta::days(1);

        let (status, body) = post_delegation(
            state,
            Some(&auth),
            json!({
                "to_user_id": users.bob.id,
                "valid_until": until,
                "workflow_filter": "[bad",
            }),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"]["message"]
                .as_str()
                .expect("message")
                .contains("invalid glob pattern")
        );
    }

    #[tokio::test]
    async fn an_anonymous_caller_is_rejected() {
        let (state, users) = test_state().await;
        let until = Utc::now() + TimeDelta::days(1);

        let (status, _) = post_delegation(
            state,
            None,
            json!({ "to_user_id": users.bob.id, "valid_until": until }),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
}
