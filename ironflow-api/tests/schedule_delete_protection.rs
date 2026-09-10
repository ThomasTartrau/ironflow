//! Integration test: handler-declared schedules cannot be deleted via the API.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::routing::delete;
use ironflow_api::routes::schedules::delete::delete_schedule;
use ironflow_api::state::AppState;
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
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

struct TestWorkflow;

impl WorkflowHandler for TestWorkflow {
    fn name(&self) -> &str {
        "deploy"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

#[tokio::test]
async fn delete_handler_schedule_returns_conflict() {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine.register(TestWorkflow).expect("register");

    let jwt_config = Arc::new(JwtConfig {
        secret: "test-handler-delete".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(1);
    let state = AppState::new(
        store.clone(),
        Arc::new(engine),
        jwt_config,
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

    let schedule = store
        .create_schedule(NewSchedule {
            workflow_name: "deploy".to_string(),
            cron_expression: "0 0 * * * *".to_string(),
            inputs: json!({}),
            source: ScheduleSource::Handler,
            created_by_user_id: Uuid::nil(),
            next_trigger_at: None,
        })
        .await
        .expect("create handler schedule");

    let token =
        AccessToken::for_user(user.id, "testuser", false, &state.jwt_config).expect("token");
    let auth = format!("Bearer {}", token.0);

    let app = Router::new()
        .route("/{id}", delete(delete_schedule))
        .with_state(state);

    let req = Request::builder()
        .uri(format!("/{}", schedule.id))
        .method("DELETE")
        .header("authorization", &auth)
        .body(Body::empty())
        .expect("build");

    let resp = app.oneshot(req).await.expect("request");
    assert_eq!(resp.status(), StatusCode::CONFLICT);
}
