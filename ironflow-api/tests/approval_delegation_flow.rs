//! End-to-end flow: alice delegates her approval power to bob, bob resolves a
//! gate assigned to alice, and the audit log names both of them.
//!
//! Everything goes through the real router built by `create_router`, a real
//! `InMemoryStore`, and the real `AuditLogSubscriber`.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{TimeDelta, Utc};
use http_body_util::BodyExt;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::{AuditLogSubscriber, Event};
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    Assignee, AuditLogFilter, EventKind, NewRun, NewStep, NewUser, RunStatus, StepKind, StepStatus,
    StepUpdate, TriggerKind, User, step_trace_id,
};
use ironflow_store::store::RunStore;
use ironflow_store::user_store::UserStore;
use serde_json::{Value as JsonValue, from_slice, json};
use tokio::sync::broadcast;
use tokio::time::sleep;
use tower::ServiceExt;
use uuid::Uuid;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;

/// A router over a store whose engine persists every event to the audit log.
fn test_app(store: Arc<InMemoryStore>) -> (Router, AppState) {
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-delegation-flow".to_string(),
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

async fn create_member(store: &Arc<InMemoryStore>, username: &str) -> User {
    let password_hash = password::hash("password123").expect("hash");
    store
        .create_user(NewUser {
            email: format!("{username}@example.com"),
            username: username.to_string(),
            password_hash,
            // The first user would otherwise become an implicit admin.
            is_admin: Some(false),
        })
        .await
        .expect("create user")
}

fn member_header(user: &User, state: &AppState) -> String {
    let token =
        AccessToken::for_user(user.id, &user.username, false, &state.jwt_config).expect("token");
    format!("Bearer {}", token.0)
}

/// Send a request and return its status plus its parsed JSON body.
async fn send(app: &Router, req: Request<Body>) -> (StatusCode, JsonValue) {
    let resp = app.clone().oneshot(req).await.expect("request");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let value = if bytes.is_empty() {
        JsonValue::Null
    } else {
        from_slice(&bytes).expect("json")
    };
    (status, value)
}

/// A run of `workflow_name` suspended on a gate assigned to `alice`.
async fn run_awaiting_alice(store: &Arc<InMemoryStore>, workflow_name: &str) -> Uuid {
    let run = store
        .create_run(NewRun {
            created_by: None,
            workflow_name: workflow_name.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .expect("create run")
        .into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("to running");
    store
        .update_run_status(run.id, RunStatus::AwaitingApproval)
        .await
        .expect("to awaiting approval");

    let step = store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "gate", 0),
            name: "gate".to_string(),
            kind: StepKind::Approval,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .expect("create step");
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Running),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("step to running");
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::AwaitingApproval),
                approval_assignee: Some(Assignee::user("alice")),
                ..StepUpdate::default()
            },
        )
        .await
        .expect("step to awaiting approval");

    run.id
}

/// Wait for the `approval_granted` audit entry of `run_id` and return its payload.
///
/// The publisher dispatches on a spawned task, so the entry lands shortly after
/// the HTTP response.
async fn await_approval_granted(store: &Arc<InMemoryStore>, run_id: Uuid) -> JsonValue {
    for _ in 0..100 {
        let page = store
            .list_audit_logs(
                AuditLogFilter {
                    event_type: Some(EventKind::ApprovalGranted),
                    run_id: Some(run_id),
                    ..AuditLogFilter::default()
                },
                1,
                10,
            )
            .await
            .expect("list audit logs");
        if let Some(entry) = page.items.first() {
            return entry.payload.clone();
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("no approval_granted audit entry was recorded for run {run_id}");
}

#[tokio::test]
async fn a_delegate_approves_for_the_delegator_until_the_delegation_is_revoked() {
    let store = Arc::new(InMemoryStore::new());
    let alice = create_member(&store, "alice").await;
    let bob = create_member(&store, "bob").await;
    let (app, state) = test_app(store.clone());
    let alice_auth = member_header(&alice, &state);
    let bob_auth = member_header(&bob, &state);

    // 1. alice delegates her approval power to bob.
    let (status, body) = send(
        &app,
        Request::builder()
            .uri("/api/v1/approval-delegations")
            .method("POST")
            .header("authorization", &alice_auth)
            .header("content-type", "application/json")
            .body(Body::from(
                json!({
                    "to_user_id": bob.id,
                    "valid_until": Utc::now() + TimeDelta::days(7),
                })
                .to_string(),
            ))
            .expect("build"),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let delegation_id = body["data"]["id"].as_str().expect("id").to_string();
    assert_eq!(body["data"]["from_user_id"], alice.id.to_string());

    // 2. bob sees the delegation he received.
    let (status, body) = send(
        &app,
        Request::builder()
            .uri("/api/v1/approval-delegations")
            .header("authorization", &bob_auth)
            .body(Body::empty())
            .expect("build"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let ids: Vec<&str> = body["data"]
        .as_array()
        .expect("data array")
        .iter()
        .map(|d| d["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, vec![delegation_id.as_str()]);

    // 3. bob resolves a gate assigned to alice.
    let first_run = run_awaiting_alice(&store, "deploy").await;
    let (status, _) = send(
        &app,
        Request::builder()
            .uri(format!("/api/v1/runs/{first_run}/approve"))
            .method("POST")
            .header("authorization", &bob_auth)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .expect("build"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    // 4. the audit log names both the delegate and the delegator.
    let payload = await_approval_granted(&store, first_run).await;
    assert_eq!(payload["approved_by"], "bob (delegated from alice)");

    // 5. alice revokes the delegation.
    let (status, _) = send(
        &app,
        Request::builder()
            .uri(format!("/api/v1/approval-delegations/{delegation_id}"))
            .method("DELETE")
            .header("authorization", &alice_auth)
            .body(Body::empty())
            .expect("build"),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // 6. bob can no longer answer for alice.
    let second_run = run_awaiting_alice(&store, "deploy").await;
    let (status, _) = send(
        &app,
        Request::builder()
            .uri(format!("/api/v1/runs/{second_run}/approve"))
            .method("POST")
            .header("authorization", &bob_auth)
            .header("content-type", "application/json")
            .body(Body::from("{}"))
            .expect("build"),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    let still_waiting = store
        .get_run(second_run)
        .await
        .expect("get run")
        .expect("run exists");
    assert_eq!(still_waiting.status.state, RunStatus::AwaitingApproval);
}
