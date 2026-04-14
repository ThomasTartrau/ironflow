//! `GET /api/v1/users` -- List all users (admin only).

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;

use ironflow_auth::extractor::Authenticated;

use crate::entities::UserResponse;
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// Query parameters for listing users.
#[derive(Debug, Deserialize)]
pub struct ListUsersQuery {
    /// Page number (1-based, defaults to 1).
    pub page: Option<u32>,
    /// Items per page (defaults to 20, max 100).
    pub per_page: Option<u32>,
}

/// List all users with pagination. Admin only.
///
/// # Errors
///
/// - 403 if the caller is not an admin
pub async fn list_users(
    auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<ListUsersQuery>,
) -> Result<impl IntoResponse, ApiError> {
    if !auth.is_admin() {
        return Err(ApiError::Forbidden);
    }

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(20).clamp(1, 100);

    let result = state.user_store.list_users(page, per_page).await?;

    let items: Vec<UserResponse> = result.items.into_iter().map(UserResponse::from).collect();

    Ok(ok_paged(items, page, per_page, result.total))
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
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_store::api_key_store::ApiKeyStore;
    use ironflow_store::entities::NewUser;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::user_store::UserStore;
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use tower::ServiceExt;
    use uuid::Uuid;

    use super::*;

    struct TestWorkflow;

    impl WorkflowHandler for TestWorkflow {
        fn name(&self) -> &str {
            "test-workflow"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret-for-user-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    fn test_state() -> AppState {
        let store = Arc::new(InMemoryStore::new());
        let user_store: Arc<dyn UserStore> = Arc::new(InMemoryStore::new());
        let api_key_store: Arc<dyn ApiKeyStore> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine
            .register(TestWorkflow)
            .expect("failed to register test workflow");
        AppState::new(
            store,
            user_store,
            api_key_store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
        )
    }

    fn make_auth_header(user_id: Uuid, is_admin: bool, state: &AppState) -> String {
        let token =
            AccessToken::for_user(user_id, "testuser", is_admin, &state.jwt_config).unwrap();
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn list_users_as_admin() {
        let state = test_state();
        let admin = state
            .user_store
            .create_user(NewUser {
                email: "admin@example.com".to_string(),
                username: "admin".to_string(),
                password_hash: "hash".to_string(),
                is_admin: None,
            })
            .await
            .unwrap();

        let auth_header = make_auth_header(admin.id, true, &state);
        let app = Router::new().route("/", get(list_users)).with_state(state);

        let req = Request::builder()
            .uri("/?page=1&per_page=20")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: JsonValue = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["data"].as_array().unwrap().len(), 1);
        assert_eq!(json["meta"]["total"], 1);
    }

    #[tokio::test]
    async fn list_users_as_member_forbidden() {
        let state = test_state();
        let member = state
            .user_store
            .create_user(NewUser {
                email: "member@example.com".to_string(),
                username: "member".to_string(),
                password_hash: "hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();

        let auth_header = make_auth_header(member.id, false, &state);
        let app = Router::new().route("/", get(list_users)).with_state(state);

        let req = Request::builder()
            .uri("/?page=1&per_page=20")
            .header("authorization", auth_header)
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }
}
