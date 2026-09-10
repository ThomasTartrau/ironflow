//! `GET /api/v1/schedules` -- List schedules with pagination.

use axum::extract::{Query, State};
use axum::response::IntoResponse;
use serde::Deserialize;

use ironflow_auth::extractor::Authenticated;

use crate::entities::ScheduleResponse;
use crate::error::ApiError;
use crate::response::ok_paged;
use crate::state::AppState;

/// Pagination query parameters.
#[derive(Debug, Deserialize)]
pub struct ListSchedulesQuery {
    /// Page number (1-based). Defaults to 1.
    #[serde(default = "default_page")]
    pub page: u32,
    /// Items per page. Defaults to 20.
    #[serde(default = "default_per_page")]
    pub per_page: u32,
}

fn default_page() -> u32 {
    1
}

fn default_per_page() -> u32 {
    20
}

/// List all schedules, paginated.
///
/// # Errors
///
/// - 401 if not authenticated
#[cfg_attr(
    feature = "openapi",
    utoipa::path(
        get,
        path = "/api/v1/schedules",
        tags = ["schedules"],
        params(
            ("page" = Option<u32>, Query, description = "Page number (1-based)"),
            ("per_page" = Option<u32>, Query, description = "Items per page"),
        ),
        responses(
            (status = 200, description = "Paginated schedules", body = Vec<ScheduleResponse>),
            (status = 401, description = "Unauthorized")
        ),
        security(("Bearer" = []))
    )
)]
pub async fn list_schedules(
    _auth: Authenticated,
    State(state): State<AppState>,
    Query(query): Query<ListSchedulesQuery>,
) -> Result<impl IntoResponse, ApiError> {
    let page = state
        .store
        .list_schedules(query.page, query.per_page)
        .await?;

    let items: Vec<ScheduleResponse> = page.items.into_iter().map(ScheduleResponse::from).collect();

    Ok(ok_paged(items, page.page, page.per_page, page.total))
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
    use ironflow_store::entities::{NewSchedule, NewUser, ScheduleSource};
    use ironflow_store::memory::InMemoryStore;
    use ironflow_store::store::Store;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::broadcast;
    use tower::ServiceExt;
    use uuid::Uuid;

    use crate::state::AppState;

    use super::*;

    struct TestWorkflow;

    impl WorkflowHandler for TestWorkflow {
        fn name(&self) -> &str {
            "deploy"
        }

        fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
            Box::pin(async move { Ok(()) })
        }
    }

    fn test_jwt_config() -> Arc<JwtConfig> {
        Arc::new(JwtConfig {
            secret: "test-secret-for-schedule-list".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        })
    }

    async fn test_state_with_user() -> (AppState, Uuid) {
        let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
        let provider = Arc::new(ClaudeCodeProvider::new());
        let mut engine = Engine::new(store.clone(), provider);
        engine.register(TestWorkflow).expect("register");
        let (event_sender, _) = broadcast::channel::<Event>(1);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            test_jwt_config(),
            "test-worker-token".to_string(),
            event_sender,
        );
        let hash = password::hash("password123").expect("hash");
        let user = store
            .create_user(NewUser {
                email: "test@example.com".to_string(),
                username: "testuser".to_string(),
                password_hash: hash,
                is_admin: None,
            })
            .await
            .expect("create user");
        (state, user.id)
    }

    fn make_auth_header(user_id: Uuid, state: &AppState) -> String {
        let token =
            AccessToken::for_user(user_id, "testuser", false, &state.jwt_config).expect("token");
        format!("Bearer {}", token.0)
    }

    #[tokio::test]
    async fn list_schedules_empty() {
        let (state, user_id) = test_state_with_user().await;
        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/", get(list_schedules))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", &auth)
            .body(Body::empty())
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(val["data"], json!([]));
        assert_eq!(val["meta"]["total"], 0);
    }

    #[tokio::test]
    async fn list_schedules_with_data() {
        let (state, user_id) = test_state_with_user().await;
        state
            .store
            .create_schedule(NewSchedule {
                workflow_name: "deploy".to_string(),
                cron_expression: "0 0 * * * *".to_string(),
                inputs: json!({}),
                source: ScheduleSource::Api,
                created_by_user_id: user_id,
                next_trigger_at: None,
            })
            .await
            .expect("create");

        let auth = make_auth_header(user_id, &state);
        let app = Router::new()
            .route("/", get(list_schedules))
            .with_state(state);

        let req = Request::builder()
            .uri("/")
            .header("authorization", &auth)
            .body(Body::empty())
            .expect("build");

        let resp = app.oneshot(req).await.expect("request");
        assert_eq!(resp.status(), StatusCode::OK);

        let body = resp.into_body().collect().await.expect("body").to_bytes();
        let val: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(val["meta"]["total"], 1);
        assert_eq!(val["data"][0]["workflow_name"], "deploy");
    }
}
