//! `GET /api/v1/users/{id}/groups` -- List a user's groups (admin only).
//!
//! `PUT /api/v1/users/{id}/groups` -- Replace a user's groups (admin only).
//!
//! Group membership restricts who may vote on an approval gate whose rule
//! lists `approver_groups`.

use std::collections::BTreeSet;

use axum::Json;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use uuid::Uuid;

use ironflow_auth::extractor::Authenticated;
use ironflow_store::error::StoreError;

use crate::entities::{UpdateUserGroupsRequest, UserGroupsResponse};
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Maximum number of groups a user may belong to.
const MAX_GROUPS: usize = 50;

/// Maximum length of a group name, in bytes.
const MAX_GROUP_LEN: usize = 64;

/// List the groups a user belongs to. Admin only.
///
/// # Errors
///
/// - 403 if the caller is not an admin
/// - 404 if the user does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/users/{id}/groups",
        tags = ["users"],
        params(
            ("id" = Uuid, Path, description = "User ID")
        ),
        responses(
            (status = 200, description = "User groups", body = UserGroupsResponse),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden (not an admin)"),
            (status = 404, description = "User not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn get_user_groups(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    if state.store.find_user_by_id(id).await?.is_none() {
        return Err(ApiError::UserNotFound(id));
    }
    let groups = state.store.list_user_groups(id).await?;

    Ok(ok(UserGroupsResponse {
        user_id: id,
        groups,
    }))
}

/// Replace the groups a user belongs to. Admin only.
///
/// Names are trimmed, deduplicated and returned sorted. An empty list removes
/// the user from every group.
///
/// # Errors
///
/// - 400 if a group name is empty, longer than 64 characters, uses a
///   character outside `[A-Za-z0-9_.-]`, or if more than 50 groups are given
/// - 403 if the caller is not an admin
/// - 404 if the user does not exist
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        put,
        path = "/api/v1/users/{id}/groups",
        tags = ["users"],
        params(
            ("id" = Uuid, Path, description = "User ID")
        ),
        request_body(content = UpdateUserGroupsRequest, description = "Complete new set of groups"),
        responses(
            (status = 200, description = "User groups replaced", body = UserGroupsResponse),
            (status = 400, description = "Invalid group name or too many groups"),
            (status = 401, description = "Unauthorized"),
            (status = 403, description = "Forbidden (not an admin)"),
            (status = 404, description = "User not found")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn update_user_groups(
    auth: Authenticated,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<UpdateUserGroupsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let groups = normalize_groups(req.groups)?;
    let groups = state
        .store
        .set_user_groups(id, groups)
        .await
        .map_err(|e| match e {
            StoreError::UserNotFound(id) => ApiError::UserNotFound(id),
            other => ApiError::Store(other),
        })?;

    Ok(ok(UserGroupsResponse {
        user_id: id,
        groups,
    }))
}

/// Trim, validate, deduplicate and sort group names.
fn normalize_groups(groups: Vec<String>) -> Result<Vec<String>, ApiError> {
    let mut normalized = BTreeSet::new();
    for group in groups {
        let group = group.trim();
        if group.is_empty() {
            return Err(ApiError::BadRequest(
                "group name must not be empty".to_string(),
            ));
        }
        if group.len() > MAX_GROUP_LEN {
            return Err(ApiError::BadRequest(format!(
                "group name must be at most {MAX_GROUP_LEN} characters"
            )));
        }
        let valid = group
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'));
        if !valid {
            return Err(ApiError::BadRequest(format!(
                "group name {group:?} may only contain letters, digits, '_', '.' and '-'"
            )));
        }
        normalized.insert(group.to_string());
    }
    if normalized.len() > MAX_GROUPS {
        return Err(ApiError::BadRequest(format!(
            "a user may belong to at most {MAX_GROUPS} groups"
        )));
    }
    Ok(normalized.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::{NewUser, User};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::{Value as JsonValue, from_slice, json, to_string};
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    fn test_state() -> AppState {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let engine = Engine::new(store.clone(), provider);
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-group-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            Arc::new(engine),
            jwt_config,
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn auth_header(user_id: Uuid, is_admin: bool, state: &AppState) -> String {
        let token =
            AccessToken::for_user(user_id, "testuser", is_admin, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    async fn user(state: &AppState, username: &str) -> User {
        state
            .store
            .create_user(NewUser {
                email: format!("{username}@example.com"),
                username: username.to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap()
    }

    fn app(state: AppState) -> Router {
        Router::new()
            .route("/{id}/groups", get(get_user_groups).put(update_user_groups))
            .with_state(state)
    }

    async fn put_groups(
        state: &AppState,
        auth: &str,
        id: Uuid,
        body: JsonValue,
    ) -> (StatusCode, JsonValue) {
        let req = Request::builder()
            .uri(format!("/{id}/groups"))
            .method("PUT")
            .header("content-type", "application/json")
            .header("authorization", auth)
            .body(Body::from(to_string(&body).unwrap()))
            .unwrap();
        send(state, req).await
    }

    async fn get_groups(state: &AppState, auth: &str, id: Uuid) -> (StatusCode, JsonValue) {
        let req = Request::builder()
            .uri(format!("/{id}/groups"))
            .method("GET")
            .header("authorization", auth)
            .body(Body::empty())
            .unwrap();
        send(state, req).await
    }

    async fn send(state: &AppState, req: Request<Body>) -> (StatusCode, JsonValue) {
        let resp = app(state.clone()).oneshot(req).await.unwrap();
        let status = resp.status();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json = from_slice(&body).unwrap_or(JsonValue::Null);
        (status, json)
    }

    #[tokio::test]
    async fn put_then_get_roundtrips_the_groups() {
        let state = test_state();
        let alice = user(&state, "alice").await;
        let admin = auth_header(Uuid::now_v7(), true, &state);

        let (status, body) = put_groups(
            &state,
            &admin,
            alice.id,
            json!({"groups": ["finance", "sre"]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["user_id"], json!(alice.id));
        assert_eq!(body["data"]["groups"], json!(["finance", "sre"]));

        let (status, body) = get_groups(&state, &admin, alice.id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["groups"], json!(["finance", "sre"]));
    }

    #[tokio::test]
    async fn groups_are_trimmed_deduplicated_and_sorted() {
        let state = test_state();
        let alice = user(&state, "alice").await;
        let admin = auth_header(Uuid::now_v7(), true, &state);

        let body = json!({"groups": ["sre", " finance ", "sre", "ops.eu-1"]});
        let (status, body) = put_groups(&state, &admin, alice.id, body).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body["data"]["groups"],
            json!(["finance", "ops.eu-1", "sre"])
        );
    }

    #[tokio::test]
    async fn an_empty_list_clears_the_groups() {
        let state = test_state();
        let alice = user(&state, "alice").await;
        let admin = auth_header(Uuid::now_v7(), true, &state);
        let body = json!({"groups": ["finance"]});
        let (status, _) = put_groups(&state, &admin, alice.id, body).await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = put_groups(&state, &admin, alice.id, json!({"groups": []})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["data"]["groups"], json!([]));

        let (_, body) = get_groups(&state, &admin, alice.id).await;
        assert_eq!(body["data"]["groups"], json!([]));
    }

    #[tokio::test]
    async fn a_member_is_forbidden() {
        let state = test_state();
        let alice = user(&state, "alice").await;
        let member = auth_header(alice.id, false, &state);

        let body = json!({"groups": ["finance"]});
        let (status, _) = put_groups(&state, &member, alice.id, body).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = get_groups(&state, &member, alice.id).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_unknown_user_is_not_found() {
        let state = test_state();
        let admin = auth_header(Uuid::now_v7(), true, &state);
        let unknown = Uuid::now_v7();

        let body = json!({"groups": ["finance"]});
        let (status, _) = put_groups(&state, &admin, unknown, body).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        let (status, _) = get_groups(&state, &admin, unknown).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn invalid_group_names_are_rejected() {
        let state = test_state();
        let alice = user(&state, "alice").await;
        let admin = auth_header(Uuid::now_v7(), true, &state);

        let too_long = "g".repeat(MAX_GROUP_LEN + 1);
        let invalid = [
            "",
            "   ",
            "fin ance",
            "fin/ance",
            "équipe",
            too_long.as_str(),
        ];
        for group in invalid {
            let body = json!({"groups": [group]});
            let (status, _) = put_groups(&state, &admin, alice.id, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "group {group:?}");
        }

        let (_, body) = get_groups(&state, &admin, alice.id).await;
        assert_eq!(body["data"]["groups"], json!([]));
    }

    #[tokio::test]
    async fn too_many_groups_are_rejected() {
        let state = test_state();
        let alice = user(&state, "alice").await;
        let admin = auth_header(Uuid::now_v7(), true, &state);

        let groups: Vec<String> = (0..=MAX_GROUPS).map(|i| format!("group-{i}")).collect();
        let (status, _) = put_groups(&state, &admin, alice.id, json!({ "groups": groups })).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let groups: Vec<String> = (0..MAX_GROUPS).map(|i| format!("group-{i}")).collect();
        let (status, _) = put_groups(&state, &admin, alice.id, json!({ "groups": groups })).await;
        assert_eq!(status, StatusCode::OK);
    }
}
