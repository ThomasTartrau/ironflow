use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::broadcast;
use tokio::time::sleep;
use uuid::Uuid;

use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::{AuditLogSubscriber, Event};
use ironflow_store::audit_log_store::AuditLogStore;
use ironflow_store::entities::{
    AuditLogEntry, AuditLogFilter, EventKind, NewUser, Run, RunStatus, TriggerKind,
};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::NewRun;
use ironflow_store::store::Store;

use crate::state::AppState;

/// An `AppState` over an in-memory store, without artifact storage.
pub(crate) fn test_state() -> AppState {
    let store = Arc::new(InMemoryStore::new());
    let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
    state_with_engine(store, engine, false)
}

/// An `AppState` whose engine persists every event to the store's audit log.
pub(crate) fn test_state_with_audit_log(store: Arc<InMemoryStore>) -> AppState {
    let engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()));
    state_with_engine(store, engine, true)
}

/// An `AppState` over `store` and `engine`, without artifact storage.
///
/// When `audit_log` is set, the engine first subscribes an
/// [`AuditLogSubscriber`] on every event, as the reference server does.
pub(crate) fn state_with_engine(
    store: Arc<InMemoryStore>,
    mut engine: Engine,
    audit_log: bool,
) -> AppState {
    if audit_log {
        engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
    }
    let jwt_config = Arc::new(JwtConfig {
        secret: "test-secret".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(1);

    AppState::new(
        store,
        Arc::new(engine),
        jwt_config,
        "test-worker-token".to_string(),
        event_sender,
    )
}

/// The audit entries of `kind` recorded so far, for `run_id` when given.
pub(crate) async fn audit_entries(
    store: &InMemoryStore,
    kind: EventKind,
    run_id: Option<Uuid>,
) -> Vec<AuditLogEntry> {
    let filter = AuditLogFilter {
        event_type: Some(kind),
        run_id,
        ..AuditLogFilter::default()
    };
    store
        .list_audit_logs(filter, 1, 10)
        .await
        .expect("list audit logs")
        .items
}

/// Wait for the audit log to hold an entry of `kind`, for `run_id` when
/// given, and return it.
///
/// The engine publishes to its subscribers on a spawned task, so the entry
/// lands shortly after the HTTP response.
pub(crate) async fn await_audit_entry(
    store: &InMemoryStore,
    kind: EventKind,
    run_id: Option<Uuid>,
) -> AuditLogEntry {
    for _ in 0..100 {
        if let Some(entry) = audit_entries(store, kind, run_id).await.into_iter().next() {
            return entry;
        }
        sleep(Duration::from_millis(10)).await;
    }
    panic!("no {kind} audit entry was recorded for run {run_id:?}");
}

/// A `Bearer` header for a non-admin user of this state.
pub(crate) async fn auth_header(state: &AppState) -> String {
    create_user_auth_header(state, "testuser", false).await
}

/// A `Bearer` header for the stored user `username`, created on first use.
///
/// The token must belong to a user in the store: a token whose user does not
/// exist is rejected. A second call with the same `username` reuses the user.
pub(crate) async fn create_user_auth_header(
    state: &AppState,
    username: &str,
    is_admin: bool,
) -> String {
    let existing = state
        .store
        .find_user_by_username(username)
        .await
        .expect("look up user");
    let user = match existing {
        Some(user) => user,
        None => state
            .store
            .create_user(NewUser {
                email: format!("{username}@test.com"),
                username: username.to_string(),
                password_hash: "argon2hash".to_string(),
                is_admin: Some(is_admin),
            })
            .await
            .expect("create user"),
    };
    let token = AccessToken::for_user(user.id, &user.username, user.is_admin, &state.jwt_config)
        .expect("issue token");
    format!("Bearer {}", token.0)
}

/// A pending run in this state's store.
pub(crate) async fn create_run(state: &AppState) -> Run {
    state
        .store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .expect("create run")
        .into_run()
}

pub(crate) async fn create_terminal_run(store: &dyn Store, name: &str, status: RunStatus) -> Run {
    let run = store
        .create_run(NewRun {
            created_by: None,
            workflow_name: name.to_string(),
            trigger: TriggerKind::Manual,
            payload: json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            concurrency_key: None,
            priority: 0,
            concurrency_limits: Vec::new(),
            max_cost_usd: None,
            worker_tags: Vec::new(),
        })
        .await
        .unwrap()
        .into_run();
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    store.update_run_status(run.id, status).await.unwrap();
    store.get_run(run.id).await.unwrap().unwrap()
}
