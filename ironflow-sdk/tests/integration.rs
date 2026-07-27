//! Integration tests: SDK against a real ironflow-api server (in-memory store).

use std::sync::Arc;
use std::time::Duration;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ClientConfig;
use ironflow_store::entities::NewUser;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::store::Store;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;

struct DeployWorkflow;

impl WorkflowHandler for DeployWorkflow {
    fn name(&self) -> &str {
        "deploy"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

struct BuildWorkflow;

impl WorkflowHandler for BuildWorkflow {
    fn name(&self) -> &str {
        "build"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

fn jwt_config() -> Arc<JwtConfig> {
    Arc::new(JwtConfig {
        secret: "integration-test-secret".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    })
}

/// Spawn a real HTTP server on a random port and return (base_url, jwt_token).
///
/// Creates a test user in the in-memory store so that `/auth/me` works.
async fn spawn_server() -> (String, String) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine.register(DeployWorkflow).unwrap();
    engine.register(BuildWorkflow).unwrap();

    let jwt_cfg = jwt_config();
    let (event_sender, _) = broadcast::channel::<Event>(16);

    let hash = password::hash("test-password").unwrap();
    let user = store
        .create_user(NewUser {
            email: "integration@test.local".to_string(),
            username: "integration-test".to_string(),
            password_hash: hash,
            is_admin: Some(true),
        })
        .await
        .unwrap();

    let state = AppState::new(
        store,
        Arc::new(engine),
        jwt_cfg.clone(),
        "test-worker-token".to_string(),
        event_sender,
    );

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
    let token = AccessToken::for_user(user.id, "integration-test", true, &jwt_cfg).unwrap();

    (base_url, token.0)
}

fn make_client(base_url: &str, token: &str) -> IronflowClient {
    let config = ClientConfig {
        base_url: base_url.to_string(),
        api_key: token.to_string(),
        timeout: Duration::from_secs(10),
    };
    IronflowClient::from_config(config)
}

#[tokio::test]
async fn health_check() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    client.health_check().await.unwrap();
}

#[tokio::test]
async fn list_runs_returns_empty() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let response = client.list_runs().await.unwrap();
    assert!(response.data.is_empty());
}

#[tokio::test]
async fn list_runs_with_filter() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let filter = ironflow_sdk::client::ListRunsFilter {
        workflow: Some("deploy"),
        page: Some(1),
        per_page: Some(10),
        ..Default::default()
    };
    let response = client.list_runs_filtered(&filter).await.unwrap();
    assert!(response.data.is_empty());
}

#[tokio::test]
async fn list_workflows_returns_registered() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let response = client.list_workflows().await.unwrap();
    let names: Vec<&str> = response.data.iter().map(|w| w.name.as_str()).collect();
    assert!(names.contains(&"deploy"));
    assert!(names.contains(&"build"));
}

#[tokio::test]
async fn get_workflow_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let response = client.get_workflow("deploy").await.unwrap();
    assert_eq!(response.data.name, "deploy");
}

#[tokio::test]
async fn get_workflow_not_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let err = client.get_workflow("nonexistent").await.unwrap_err();
    assert!(err.is_api_error());
    assert_eq!(err.status(), Some(404));
}

#[tokio::test]
async fn get_stats() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let response = client.get_stats().await.unwrap();
    assert_eq!(response.data.total_runs, 0);
}

#[tokio::test]
async fn get_me() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let response = client.me().await.unwrap();
    assert_eq!(response.data.username, "integration-test");
    assert!(response.data.is_admin);
}

#[tokio::test]
async fn get_run_not_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let fake_id = Uuid::now_v7();
    let err = client.get_run(fake_id).await.unwrap_err();
    assert!(err.is_api_error());
    assert_eq!(err.status(), Some(404));
}

#[tokio::test]
async fn create_and_get_run() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let mut payload = serde_json::Map::new();
    payload.insert("env".to_string(), serde_json::json!("staging"));

    let request = ironflow_sdk::types::CreateRunRequest {
        workflow: "deploy".to_string(),
        payload: Some(payload),
        labels: None,
        scheduled_at: None,
        max_retries: Some(2),
        max_cost_usd: None,
    };

    let created = client.create_run(&request).await.unwrap();
    assert_eq!(created.data.workflow_name, "deploy");
    assert_eq!(created.data.max_retries, 2);

    let fetched = client.get_run(created.data.id).await.unwrap();
    assert_eq!(fetched.data.run.id, created.data.id);
}

#[tokio::test]
async fn create_run_with_max_cost_usd() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let request = ironflow_sdk::types::CreateRunRequest {
        workflow: "deploy".to_string(),
        payload: None,
        labels: None,
        scheduled_at: None,
        max_retries: None,
        max_cost_usd: Some(2.5),
    };

    let created = client.create_run(&request).await.unwrap();
    assert_eq!(created.data.max_cost_usd, Some(2.5));
}

#[tokio::test]
async fn create_run_rejects_negative_max_cost_usd() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let request = ironflow_sdk::types::CreateRunRequest {
        workflow: "deploy".to_string(),
        payload: None,
        labels: None,
        scheduled_at: None,
        max_retries: None,
        max_cost_usd: Some(-1.0),
    };

    let err = client.create_run(&request).await.unwrap_err();
    assert_eq!(err.status(), Some(400));
}

#[tokio::test]
async fn create_run_unknown_workflow() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let request = ironflow_sdk::types::CreateRunRequest {
        workflow: "does-not-exist".to_string(),
        payload: None,
        labels: None,
        scheduled_at: None,
        max_retries: None,
        max_cost_usd: None,
    };

    let err = client.create_run(&request).await.unwrap_err();
    assert!(err.is_api_error());
}

#[tokio::test]
async fn unauthorized_without_token() {
    let (base_url, _) = spawn_server().await;
    let client = make_client(&base_url, "invalid-token");

    let err = client.list_runs().await.unwrap_err();
    assert!(err.is_api_error());
    assert_eq!(err.status(), Some(401));
}
