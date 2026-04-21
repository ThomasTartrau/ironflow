//! `GET /api/v1/auth/me` — Get the current authenticated user profile.

use axum::extract::State;
use axum::response::IntoResponse;

use ironflow_auth::extractor::AuthenticatedUser;

use crate::entities::MeResponse;
use crate::error::ApiError;
use crate::response::ok;
use crate::state::AppState;

/// Return the current user's profile.
///
/// # Errors
///
/// - 401 if no valid token is provided
/// - 404 if the user no longer exists in the store
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/auth/me",
        tags = ["auth"],
        responses(
            (status = 200, description = "Current user profile", body = MeResponse),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn me(
    State(state): State<AppState>,
    user: AuthenticatedUser,
) -> Result<impl IntoResponse, ApiError> {
    let stored_user = state
        .store
        .find_user_by_id(user.user_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    Ok(ok(MeResponse {
        user_id: stored_user.id,
        email: stored_user.email,
        username: stored_user.username,
        is_admin: stored_user.is_admin,
    }))
}

#[cfg(test)]
mod tests {
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use http_body_util::BodyExt;
    use ironflow_auth::jwt::{AccessToken, JwtConfig};
    use ironflow_auth::password;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_engine::context::WorkflowContext;
    use ironflow_engine::engine::Engine;
    use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
    use ironflow_engine::notify::Event;
    use ironflow_store::entities::NewUser;
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::Value as JsonValue;
    use std::sync::Arc;
    use tokio::sync::broadcast;
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
            secret: "test-secret-for-auth-tests".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    fn test_state() -> AppState {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine
            .register(TestWorkflow)
            .expect("failed to register test workflow");
        let (event_sender, _) = broadcast::channel::<Event>(1);
        AppState::new(
            store,
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
            event_sender,
        )
    }

    fn make_auth_header(user_id: Uuid, state: &AppState) -> String {
        let token = AccessToken::for_user(user_id, "testuser", false, &state.jwt_config)
            .expect("failed to create token");
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn me_success() {
        let state = test_state();
        let _user_id = Uuid::now_v7();
        let hash = password::hash("password123").expect("failed to hash password");
        let user = state
            .store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: hash,
                is_admin: None,
            })
            .await
            .expect("failed to create user");

        let auth_header = make_auth_header(user.id, &state);
        let app = Router::new().route("/", get(me)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("GET")
            .header("authorization", auth_header)
            .body(Body::empty())
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp
            .into_body()
            .collect()
            .await
            .expect("failed to collect body")
            .to_bytes();
        let json_val: JsonValue = serde_json::from_slice(&body).expect("failed to parse json");
        assert_eq!(json_val["data"]["email"], "test@example.com");
        assert_eq!(json_val["data"]["username"], "testuser");
    }

    #[tokio::test]
    async fn me_user_not_found() {
        let state = test_state();
        let user_id = Uuid::now_v7();
        let auth_header = make_auth_header(user_id, &state);

        let app = Router::new().route("/", get(me)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("GET")
            .header("authorization", auth_header)
            .body(Body::empty())
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn me_without_token() {
        let state = test_state();
        let app = Router::new().route("/", get(me)).with_state(state);

        let req = Request::builder()
            .uri("/")
            .method("GET")
            .body(Body::empty())
            .expect("failed to build request");

        let resp = app.oneshot(req).await.expect("request failed");
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
