//! End-to-end signal flow through the real API router.
//!
//! A run waits on `ctx.wait_for_signal`, a signal is posted to
//! `POST /api/v1/signals`, and the run resumes in-process (`ExecutionMode::Local`)
//! with the payload. Everything goes through the real router built by
//! `create_router`, a real `InMemoryStore` and real JWTs.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_engine::signal::Signal;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_slice, json};
use tokio::sync::broadcast;
use tokio::time::{sleep, timeout};
use tower::ServiceExt;
use uuid::Uuid;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;

const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct PipelineFinished {
    status: String,
}

impl Signal for PipelineFinished {
    const NAME: &'static str = "ci.pipeline_finished";
}

struct WaitCi {
    received: Arc<Mutex<Vec<String>>>,
}

impl WorkflowHandler for WaitCi {
    fn name(&self) -> &str {
        "wait-ci"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let finished = ctx
                .wait_for_signal::<PipelineFinished>("wait-ci", "abc123", Duration::from_secs(60))
                .await?;
            if let Some(finished) = finished {
                self.received
                    .lock()
                    .expect("received lock")
                    .push(finished.status);
            }
            Ok(())
        })
    }
}

fn test_app(store: Arc<InMemoryStore>, received: Arc<Mutex<Vec<String>>>) -> (Router, AppState) {
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine
        .register(WaitCi { received })
        .expect("register handler");
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-signal-flow".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(1);
    let state = AppState::new(
        store,
        Arc::new(engine),
        jwt_config,
        "test-worker-token".to_string(),
        event_sender,
    );
    // The limiter keys on the peer address, which `oneshot` does not provide.
    let config = RouterConfig {
        rate_limit_auth: None,
        rate_limit_general: None,
        ..RouterConfig::default()
    };
    (create_router(state.clone(), config), state)
}

fn admin_header(state: &AppState) -> String {
    let token =
        AccessToken::for_user(Uuid::now_v7(), "admin", true, &state.jwt_config).expect("token");
    format!("Bearer {}", token.0)
}

#[tokio::test]
async fn signal_resumes_run_through_router() {
    // Built outside the timeout: `AppState::new` loads the TLS root store
    // synchronously, which takes seconds on a loaded CI runner and would eat
    // the budget meant for the signal round trip.
    let store = Arc::new(InMemoryStore::new());
    let received = Arc::new(Mutex::new(Vec::new()));
    let (app, state) = test_app(store.clone(), received.clone());

    timeout(TEST_TIMEOUT, async {
        let result = state
            .engine
            .run_handler("wait-ci", TriggerKind::Manual, json!({}))
            .await
            .expect("handler suspends on the signal");
        assert_eq!(result.run.status.state, RunStatus::Sleeping);
        let run_id = result.run.id;

        let body = json!({
            "name": "ci.pipeline_finished",
            "key": "abc123",
            "payload": {"status": "success"}
        });
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/signals")
            .header("authorization", admin_header(&state))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("request");
        let resp = app.oneshot(req).await.expect("response");
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let value: Value = from_slice(&bytes).expect("json");
        assert_eq!(value["data"]["duplicate"], false);
        assert_eq!(value["data"]["resumed"][0]["run_id"], json!(run_id));

        loop {
            let run = store
                .get_run(run_id)
                .await
                .expect("get run")
                .expect("run exists");
            if run.status.state == RunStatus::Completed {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(*received.lock().unwrap(), vec!["success".to_string()]);
    })
    .await
    .expect("test timed out");
}
