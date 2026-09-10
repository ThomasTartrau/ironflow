//! Tests for subscribe_run_events and subscribe_global_events.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures_util::StreamExt;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::engine::Engine;
use ironflow_engine::notify::{Event, WorkflowEvent, WorkflowEventBus};
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ClientConfig;
use ironflow_store::entities::NewUser;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, TriggerKind};
use ironflow_store::store::Store;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;

fn jwt_config() -> Arc<JwtConfig> {
    Arc::new(JwtConfig {
        secret: "sse-test-secret".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    })
}

async fn spawn_server_with_bus() -> (String, String, Arc<dyn Store>, WorkflowEventBus) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let engine = Engine::new(store.clone(), provider);

    let jwt_cfg = jwt_config();
    let (event_sender, _) = broadcast::channel::<Event>(16);

    let hash = password::hash("test-password").unwrap();
    let user = store
        .create_user(NewUser {
            email: "sse-test@test.local".to_string(),
            username: "sse-test".to_string(),
            password_hash: hash,
            is_admin: Some(true),
        })
        .await
        .unwrap();

    let bus = WorkflowEventBus::new();
    let state = AppState::new(
        store.clone(),
        Arc::new(engine),
        jwt_cfg.clone(),
        "test-worker-token".to_string(),
        event_sender,
    )
    .with_event_bus(bus.clone());

    let config = RouterConfig {
        rate_limit_auth: None,
        rate_limit_general: None,
        ..RouterConfig::default()
    };
    let router = create_router(state, config);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let base_url = format!("http://{addr}");
    let token = AccessToken::for_user(user.id, "sse-test", true, &jwt_cfg).unwrap();

    (base_url, token.0, store, bus)
}

fn make_client(base_url: &str, token: &str) -> IronflowClient {
    IronflowClient::from_config(ClientConfig {
        base_url: base_url.to_string(),
        api_key: token.to_string(),
        timeout: Duration::from_secs(10),
    })
}

async fn create_run(store: &dyn Store) -> Uuid {
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "test".to_string(),
            trigger: TriggerKind::Manual,
            payload: serde_json::json!({}),
            max_retries: 0,
            handler_version: None,
            labels: HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .unwrap()
        .into_run()
        .id
}

// ── Row 9: subscribe_run_events receives events ──────────────────

#[tokio::test]
async fn subscribe_run_events_receives_step_started() {
    let (base_url, token, store, bus) = spawn_server_with_bus().await;
    let client = make_client(&base_url, &token);
    let run_id = create_run(&*store).await;

    let mut stream = client.subscribe_run_events(run_id).await.unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    bus.publish(
        run_id,
        WorkflowEvent::StepStarted {
            step_name: "deploy".to_string(),
            step_index: 0,
            timestamp: Utc::now(),
        },
    );

    let event = tokio::time::timeout(Duration::from_secs(5), stream.next())
        .await
        .expect("timeout waiting for SSE event")
        .expect("stream ended unexpectedly")
        .expect("SSE parse error");

    assert_eq!(event.event_type, "step_started");
    assert!(event.data.contains("deploy"));
}

// ── Row 10: subscribe_global_events receives events ──────────────

#[tokio::test]
async fn subscribe_global_events_receives_events() {
    let (base_url, token, _store, _bus) = spawn_server_with_bus().await;
    let client = make_client(&base_url, &token);

    let stream = client.subscribe_global_events().await;
    assert!(
        stream.is_ok(),
        "subscribe_global_events should connect successfully"
    );
}
