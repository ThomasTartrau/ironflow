//! `POST /api/v1/runs/:id/cancel` cancels the sub-workflow runs below the run,
//! and `GET /api/v1/runs/:id` tells how many a cancellation would reach.
//!
//! Everything goes through the real router built by `create_router`, a real
//! `InMemoryStore` and a real `Engine`. Test names contain `cancel_run` so
//! `cargo test -p ironflow-api cancel_run` selects them.

use std::collections::HashMap;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::PARENT_RUN_ID_LABEL;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    NewRun, NewStep, NewUser, RunStatus, StepKind, StepStatus, TriggerKind, step_trace_id,
};
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;
use serde_json::{Value, from_slice, json};
use tokio::sync::broadcast;
use tower::ServiceExt;
use uuid::Uuid;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;

/// Concurrency key held by the grand-child of the cascade test.
const KEY: &str = "issue:169";

struct App {
    router: Router,
    store: Arc<InMemoryStore>,
    auth: String,
}

async fn app() -> App {
    let store = Arc::new(InMemoryStore::new());
    let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-cancel-run-cascade".to_string(),
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
    let user = store
        .create_user(NewUser {
            email: "admin@test.com".to_string(),
            username: "admin".to_string(),
            password_hash: "argon2hash".to_string(),
            is_admin: Some(true),
        })
        .await
        .expect("create user");
    let token = AccessToken::for_user(user.id, &user.username, user.is_admin, &state.jwt_config)
        .expect("token");
    // The limiter keys on the peer address, which `oneshot` does not provide.
    let config = RouterConfig {
        rate_limit_auth: None,
        rate_limit_general: None,
        ..RouterConfig::default()
    };
    App {
        router: create_router(state, config),
        store,
        auth: format!("Bearer {}", token.0),
    }
}

impl App {
    async fn call(&self, method: &str, uri: String) -> (StatusCode, Value) {
        let req = Request::builder()
            .method(method)
            .uri(uri)
            .header("authorization", &self.auth)
            .body(Body::empty())
            .expect("request");
        let resp = self.router.clone().oneshot(req).await.expect("response");
        let status = resp.status();
        let body = resp.into_body().collect().await.expect("body").to_bytes();
        (status, from_slice(&body).unwrap_or(Value::Null))
    }

    async fn cancel(&self, id: Uuid) -> (StatusCode, Value) {
        self.call("POST", format!("/api/v1/runs/{id}/cancel")).await
    }

    /// Create a run moved to `status`: a sub-workflow child of `parent` when
    /// one is given, holding `key` when one is given.
    async fn run_in(&self, parent: Option<Uuid>, status: RunStatus, key: Option<&str>) -> Uuid {
        let labels = parent
            .map(|p| HashMap::from([(PARENT_RUN_ID_LABEL.to_string(), p.to_string())]))
            .unwrap_or_default();
        let trigger = match parent {
            Some(_) => TriggerKind::Workflow,
            None => TriggerKind::Manual,
        };
        let run = self
            .store
            .create_run(NewRun {
                created_by: None,
                workflow_name: "test".to_string(),
                trigger,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels,
                scheduled_at: None,
                idempotency_key: None,
                concurrency_key: key.map(str::to_string),
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
            })
            .await
            .expect("create run")
            .into_run();
        self.store
            .update_run_status(run.id, RunStatus::Running)
            .await
            .expect("start run");
        if status != RunStatus::Running {
            self.store
                .update_run_status(run.id, status)
                .await
                .expect("move run");
        }
        run.id
    }

    async fn status(&self, id: Uuid) -> RunStatus {
        self.store
            .get_run(id)
            .await
            .expect("get run")
            .expect("run exists")
            .status
            .state
    }
}

#[tokio::test]
async fn cancel_run_cascades_to_its_active_descendants() {
    let app = app().await;
    let root = app.run_in(None, RunStatus::Running, None).await;
    let child = app
        .run_in(Some(root), RunStatus::AwaitingApproval, None)
        .await;
    let grandchild = app.run_in(Some(child), RunStatus::Running, Some(KEY)).await;
    let finished = app.run_in(Some(root), RunStatus::Completed, None).await;
    let step = app
        .store
        .create_step(NewStep {
            run_id: grandchild,
            trace_id: step_trace_id(grandchild, "build", 0),
            name: "build".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("create step");

    let (status, body) = app.cancel(root).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["id"], root.to_string());
    assert_eq!(body["data"]["status"], "cancelled");
    assert_eq!(
        body["data"]["cancelled_descendants"],
        json!([child.to_string(), grandchild.to_string()])
    );
    for id in [root, child, grandchild] {
        assert_eq!(app.status(id).await, RunStatus::Cancelled);
    }
    assert_eq!(app.status(finished).await, RunStatus::Completed);
    let step = app
        .store
        .get_step(step.id)
        .await
        .expect("get step")
        .expect("step exists");
    assert_eq!(step.status.state, StepStatus::Skipped);
    // The grand-child released its concurrency key.
    app.run_in(None, RunStatus::Running, Some(KEY)).await;
}

#[tokio::test]
async fn cancel_run_twice_cancels_nothing_more() {
    let app = app().await;
    let root = app.run_in(None, RunStatus::Running, None).await;
    let child = app.run_in(Some(root), RunStatus::Running, None).await;

    let (_, first) = app.cancel(root).await;
    assert_eq!(
        first["data"]["cancelled_descendants"],
        json!([child.to_string()])
    );

    let (status, second) = app.cancel(root).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(second["data"]["status"], "cancelled");
    assert_eq!(second["data"]["cancelled_descendants"], json!([]));
}

#[tokio::test]
async fn cancel_run_of_a_child_leaves_its_parent_running_inline() {
    let app = app().await;
    let root = app.run_in(None, RunStatus::Running, None).await;
    let child = app.run_in(Some(root), RunStatus::Running, None).await;

    let (status, body) = app.cancel(child).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["cancelled_descendants"], json!([]));
    assert_eq!(app.status(child).await, RunStatus::Cancelled);
    assert_eq!(app.status(root).await, RunStatus::Running);
}

#[tokio::test]
async fn cancel_run_of_a_finished_or_unknown_run_is_refused() {
    let app = app().await;
    let done = app.run_in(None, RunStatus::Completed, None).await;

    let (status, body) = app.cancel(done).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(
        body["error"]["message"],
        "cannot cancel run in Completed state"
    );

    let (status, _) = app.cancel(Uuid::now_v7()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancel_run_preview_get_run_counts_active_descendants() {
    let app = app().await;
    let root = app.run_in(None, RunStatus::Running, None).await;
    let child = app.run_in(Some(root), RunStatus::Running, None).await;
    app.run_in(Some(child), RunStatus::AwaitingApproval, None)
        .await;
    app.run_in(Some(root), RunStatus::Failed, None).await;

    let (status, body) = app.call("GET", format!("/api/v1/runs/{root}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["active_descendant_count"], 2);

    let (_, body) = app.call("GET", format!("/api/v1/runs/{child}")).await;
    assert_eq!(body["data"]["active_descendant_count"], 1);
}
