//! End-to-end flow: an admin pauses and resumes a workflow over HTTP, and
//! `GET /api/v1/audit-logs` names them.
//!
//! The engine is wired like `examples/server/src/main.rs`: the
//! `AuditLogSubscriber` and the `SseBroadcaster` both listen to every event.
//! Everything goes through the real router built by `create_router` and a
//! real `InMemoryStore`.

use std::sync::Arc;
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
use ironflow_engine::notify::{AuditLogSubscriber, Event};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewUser, User};
use ironflow_store::user_store::UserStore;
use serde_json::{Value as JsonValue, from_slice};
use tokio::time::{sleep, timeout};
use tower::ServiceExt;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::sse::SseBroadcaster;
use ironflow_api::state::AppState;

/// A workflow that does nothing: the tests only pause and resume it.
struct Hello;

impl WorkflowHandler for Hello {
    fn name(&self) -> &str {
        "hello"
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

/// A router over `store`, with the engine wired like the reference server.
fn server_app(store: Arc<InMemoryStore>) -> (Router, AppState) {
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine.register(Hello).expect("register handler");
    engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
    let sse_broadcaster = SseBroadcaster::new();
    let event_sender = sse_broadcaster.sender();
    engine.subscribe(sse_broadcaster, Event::ALL);

    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret-for-workflow-pause-audit".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
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

async fn create_admin(store: &Arc<InMemoryStore>) -> User {
    store
        .create_user(NewUser {
            email: "admin@example.com".to_string(),
            username: "admin".to_string(),
            password_hash: "argon2hash".to_string(),
            is_admin: Some(true),
        })
        .await
        .expect("create admin")
}

fn admin_header(user: &User, state: &AppState) -> String {
    let token =
        AccessToken::for_user(user.id, &user.username, true, &state.jwt_config).expect("token");
    format!("Bearer {}", token.0)
}

/// Send a request and return its status plus its parsed JSON body.
async fn send(app: &Router, method: &str, uri: &str, auth: &str) -> (StatusCode, JsonValue) {
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", auth)
        .body(Body::empty())
        .expect("request");
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let value = if bytes.is_empty() {
        JsonValue::Null
    } else {
        from_slice(&bytes).expect("json")
    };
    (status, value)
}

/// The audit entries of `event_type`, read through the API.
async fn audit_entries(app: &Router, auth: &str, event_type: &str) -> Vec<JsonValue> {
    let uri = format!("/api/v1/audit-logs?event_type={event_type}");
    let (status, body) = send(app, "GET", &uri, auth).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    body["data"].as_array().expect("data array").clone()
}

/// Poll the audit log until it holds an entry of `event_type`: the engine
/// publishes to its subscribers on a spawned task, so the entry lands shortly
/// after the HTTP response.
async fn await_audit_entry(app: &Router, auth: &str, event_type: &str) -> JsonValue {
    timeout(Duration::from_secs(2), async {
        loop {
            if let Some(entry) = audit_entries(app, auth, event_type)
                .await
                .into_iter()
                .next()
            {
                return entry;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("no {event_type} entry reached the audit log"))
}

#[tokio::test]
async fn paused_workflow_appears_in_the_audit_log_with_the_admin() {
    let store = Arc::new(InMemoryStore::new());
    let (app, state) = server_app(store.clone());
    let admin = create_admin(&store).await;
    let auth = admin_header(&admin, &state);

    let (status, body) = send(&app, "POST", "/api/v1/workflows/hello/pause", &auth).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let entry = await_audit_entry(&app, &auth, "workflow_paused").await;
    assert_eq!(entry["event_type"], "workflow_paused");
    assert_eq!(entry["user_id"], admin.id.to_string());
    assert_eq!(entry["payload"]["workflow_name"], "hello");
}

#[tokio::test]
async fn resumed_workflow_appears_in_the_audit_log_with_the_admin() {
    let store = Arc::new(InMemoryStore::new());
    let (app, state) = server_app(store.clone());
    let admin = create_admin(&store).await;
    let auth = admin_header(&admin, &state);

    let (status, _) = send(&app, "POST", "/api/v1/workflows/hello/pause", &auth).await;
    assert_eq!(status, StatusCode::OK);
    let (status, body) = send(&app, "POST", "/api/v1/workflows/hello/resume", &auth).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    let entry = await_audit_entry(&app, &auth, "workflow_resumed").await;
    assert_eq!(entry["event_type"], "workflow_resumed");
    assert_eq!(entry["user_id"], admin.id.to_string());
    assert_eq!(entry["payload"]["workflow_name"], "hello");
}

#[tokio::test]
async fn resuming_a_running_workflow_records_nothing() {
    let store = Arc::new(InMemoryStore::new());
    let (app, state) = server_app(store.clone());
    let admin = create_admin(&store).await;
    let auth = admin_header(&admin, &state);

    let (status, body) = send(&app, "POST", "/api/v1/workflows/hello/resume", &auth).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");

    sleep(Duration::from_millis(100)).await;
    let resumed = audit_entries(&app, &auth, "workflow_resumed").await;
    assert!(resumed.is_empty(), "got {resumed:?}");
}
