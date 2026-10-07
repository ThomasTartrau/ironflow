//! Integration test: why Ironflow disabled a schedule is exposed by the API,
//! and resuming the schedule clears it.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use chrono::Utc;
use http_body_util::BodyExt;
use ironflow_api::routes::schedules::list::list_schedules;
use ironflow_api::routes::schedules::pause_resume::resume_schedule;
use ironflow_api::state::AppState;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_store::entities::{
    NewSchedule, NewUser, SchedulePolicy, ScheduleSource, ScheduleUpdate,
};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

const DISABLE_REASON: &str = "cannot compute next trigger: no next occurrence";

struct TestWorkflow;

impl WorkflowHandler for TestWorkflow {
    fn name(&self) -> &str {
        "deploy"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async { Ok(()) })
    }
}

/// App state, an auth header, and a schedule Ironflow disabled on an error.
async fn state_with_schedule_disabled_on_error(
    cron: &str,
) -> (AppState, Arc<dyn Store>, String, Uuid) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine.register(TestWorkflow).expect("register");

    let jwt_config = Arc::new(JwtConfig {
        secret: "test-schedule-last-error".to_string(),
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
            cron_expression: cron.to_string(),
            inputs: json!({}),
            source: ScheduleSource::Api,
            priority: 0,
            created_by_user_id: Some(user.id),
            next_trigger_at: None,
            policy: SchedulePolicy::default(),
        })
        .await
        .expect("create schedule");
    store
        .update_schedule(
            schedule.id,
            ScheduleUpdate {
                disabled_at: Some(Some(Utc::now())),
                last_error: Some(Some(DISABLE_REASON.to_string())),
                ..Default::default()
            },
        )
        .await
        .expect("disable on error");

    let token =
        AccessToken::for_user(user.id, "testuser", false, &state.jwt_config).expect("token");
    (state, store, format!("Bearer {}", token.0), schedule.id)
}

async fn body_json(resp: Response) -> Value {
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    serde_json::from_slice(&body).expect("json")
}

#[tokio::test]
async fn list_exposes_why_ironflow_disabled_a_schedule() {
    let (state, _, auth, _) = state_with_schedule_disabled_on_error("0 0 30 2 *").await;
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
    let val = body_json(resp).await;
    assert_eq!(val["data"][0]["last_error"], DISABLE_REASON);
    assert!(val["data"][0]["disabled_at"].is_string());
    assert!(val["data"][0]["next_trigger_at"].is_null());
}

#[tokio::test]
async fn resume_clears_why_ironflow_disabled_a_schedule() {
    let (state, store, auth, schedule_id) =
        state_with_schedule_disabled_on_error("0 * * * *").await;
    let app = Router::new()
        .route("/{id}/resume", post(resume_schedule))
        .with_state(state);

    let req = Request::builder()
        .uri(format!("/{schedule_id}/resume"))
        .method("POST")
        .header("authorization", &auth)
        .body(Body::empty())
        .expect("build");
    let resp = app.oneshot(req).await.expect("request");

    assert_eq!(resp.status(), StatusCode::OK);
    let val = body_json(resp).await;
    assert!(val["data"]["disabled_at"].is_null());
    assert!(val["data"]["last_error"].is_null());
    assert!(val["data"]["next_trigger_at"].is_string());

    let stored = store
        .find_schedule_by_id(schedule_id)
        .await
        .expect("find")
        .expect("exists");
    assert!(stored.is_active());
    assert!(stored.last_error.is_none());
}

#[tokio::test]
async fn resume_keeps_the_schedule_disabled_when_its_cron_still_fails() {
    let (state, store, auth, schedule_id) =
        state_with_schedule_disabled_on_error("0 0 30 2 *").await;
    let app = Router::new()
        .route("/{id}/resume", post(resume_schedule))
        .with_state(state);

    let req = Request::builder()
        .uri(format!("/{schedule_id}/resume"))
        .method("POST")
        .header("authorization", &auth)
        .body(Body::empty())
        .expect("build");
    let resp = app.oneshot(req).await.expect("request");

    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let stored = store
        .find_schedule_by_id(schedule_id)
        .await
        .expect("find")
        .expect("exists");
    assert!(!stored.is_active(), "never active without next trigger");
    assert_eq!(stored.last_error.as_deref(), Some(DISABLE_REASON));
}
