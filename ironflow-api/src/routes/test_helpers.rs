use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;
use tokio::sync::broadcast;
use uuid::Uuid;

use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::Event;
use ironflow_store::entities::{Run, RunStatus, TriggerKind};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::NewRun;
use ironflow_store::store::Store;

use crate::state::AppState;

/// An `AppState` over an in-memory store, without artifact storage.
pub(crate) fn test_state() -> AppState {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let engine = Arc::new(Engine::new(store.clone(), provider));
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
        engine,
        jwt_config,
        "test-worker-token".to_string(),
        event_sender,
    )
}

/// A `Bearer` header for a non-admin user of this state.
pub(crate) fn auth_header(state: &AppState) -> String {
    let token = AccessToken::for_user(Uuid::now_v7(), "testuser", false, &state.jwt_config)
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
            max_cost_usd: None,
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
            max_cost_usd: None,
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
