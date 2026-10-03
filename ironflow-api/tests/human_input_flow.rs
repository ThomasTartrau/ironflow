//! End-to-end flow: a handler asks for a typed human input, the run suspends,
//! the answer is posted through the API, the run resumes and completes, and a
//! second answer is refused.
//!
//! Everything goes through the real router built by `create_router`, a real
//! `InMemoryStore` and a real `Engine` with a registered handler.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::HumanInputConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewUser, RunStatus, StepKind, StepStatus, TriggerKind};
use ironflow_store::store::RunStore;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, from_slice, json};
use tokio::sync::broadcast;
use tokio::time::{sleep, timeout};
use tower::ServiceExt;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;

/// Wall-clock budget for the whole flow.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// Asks for [`Answers`] and records what it received.
struct Clarify {
    received: Arc<Mutex<Vec<String>>>,
}

impl WorkflowHandler for Clarify {
    fn name(&self) -> &str {
        "clarify"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let answers: Answers = ctx
                .human_input("clarify", HumanInputConfig::new("Answer the questions"))
                .await?;
            self.received
                .lock()
                .expect("received lock")
                .extend(answers.answers);
            Ok(())
        })
    }
}

fn test_app(store: Arc<InMemoryStore>, received: Arc<Mutex<Vec<String>>>) -> (Router, AppState) {
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine
        .register(Clarify { received })
        .expect("register handler");
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-human-input-flow".to_string(),
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

async fn admin_header(state: &AppState) -> String {
    let user = state
        .store
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
    format!("Bearer {}", token.0)
}

/// POST `body` to `uri` and return the status plus the parsed JSON body.
async fn post(app: &Router, auth: &str, uri: &str, body: &Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("authorization", auth)
        .header("content-type", "application/json")
        .body(Body::from(body.to_string()))
        .expect("request");
    let resp = app.clone().oneshot(req).await.expect("response");
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
async fn human_input_end_to_end_submit_then_conflict() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let received = Arc::new(Mutex::new(Vec::new()));
        let (app, state) = test_app(store.clone(), received.clone());
        let auth = admin_header(&state).await;

        // The handler suspends on the input.
        let result = state
            .engine
            .run_handler("clarify", TriggerKind::Manual, json!({}))
            .await
            .expect("run suspends");
        let run_id = result.run.id;
        assert_eq!(result.run.status.state, RunStatus::AwaitingApproval);

        let step = store
            .list_steps(run_id)
            .await
            .expect("list steps")
            .into_iter()
            .find(|s| s.kind == StepKind::HumanInput)
            .expect("a human input step");
        assert_eq!(step.status.state, StepStatus::AwaitingApproval);

        let uri = format!("/api/v1/runs/{run_id}/steps/{}/input", step.id);
        let answer = json!({"answers": ["staging", "eu-west"]});

        let (status, body) = post(&app, &auth, &uri, &answer).await;
        assert_eq!(status, StatusCode::OK, "body: {body}");

        // The run resumes in the background and completes with the answer.
        loop {
            let run = store.get_run(run_id).await.unwrap().unwrap();
            if run.status.state == RunStatus::Completed {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(
            received.lock().expect("received lock").clone(),
            vec!["staging".to_string(), "eu-west".to_string()]
        );

        let (status, body) = post(&app, &auth, &uri, &answer).await;
        assert_eq!(status, StatusCode::CONFLICT, "body: {body}");
        assert_eq!(body["error"]["code"], "CONFLICT");
    })
    .await
    .expect("test timed out");
}
