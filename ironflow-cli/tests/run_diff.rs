//! Integration tests for `run diff`.

use std::collections::HashMap;
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
use ironflow_store::models::{NewRun, TriggerKind};
use ironflow_store::store::Store;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;

use ironflow_cli::commands;
use ironflow_cli::commands::run::{RunArgs, RunCommands};

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

async fn spawn_server_with_store() -> (String, String, Arc<dyn Store>) {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine.register(DeployWorkflow).unwrap();
    engine.register(BuildWorkflow).unwrap();

    let jwt_cfg = Arc::new(JwtConfig {
        secret: "diff-test-secret".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    });
    let (event_sender, _) = broadcast::channel::<Event>(16);

    let hash = password::hash("test-password").unwrap();
    let user = store
        .create_user(NewUser {
            email: "diff-test@test.local".to_string(),
            username: "diff-test".to_string(),
            password_hash: hash,
            is_admin: Some(true),
        })
        .await
        .unwrap();

    let state = AppState::new(
        store.clone(),
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
    let token = AccessToken::for_user(user.id, "diff-test", true, &jwt_cfg).unwrap();

    (base_url, token.0, store)
}

fn make_client(base_url: &str, token: &str) -> IronflowClient {
    let config = ClientConfig {
        base_url: base_url.to_string(),
        api_key: token.to_string(),
        timeout: Duration::from_secs(10),
    };
    IronflowClient::from_config(config)
}

async fn create_run(store: &Arc<dyn Store>, workflow: &str) -> Uuid {
    store
        .create_run(NewRun {
            created_by: None,
            workflow_name: workflow.to_string(),
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
        .into_run()
        .id
}

#[tokio::test]
async fn run_diff_same_workflow_succeeds() {
    let (base_url, token, store) = spawn_server_with_store().await;
    let client = make_client(&base_url, &token);

    let run_a = create_run(&store, "deploy").await;
    let run_b = create_run(&store, "deploy").await;

    let args = RunArgs {
        command: RunCommands::Diff { run_a, run_b },
    };
    commands::run::execute(&client, &args, false, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_diff_json_mode() {
    let (base_url, token, store) = spawn_server_with_store().await;
    let client = make_client(&base_url, &token);

    let run_a = create_run(&store, "deploy").await;
    let run_b = create_run(&store, "deploy").await;

    let args = RunArgs {
        command: RunCommands::Diff { run_a, run_b },
    };
    commands::run::execute(&client, &args, true, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_diff_different_workflows_fails() {
    let (base_url, token, store) = spawn_server_with_store().await;
    let client = make_client(&base_url, &token);

    let run_a = create_run(&store, "deploy").await;
    let run_b = create_run(&store, "build").await;

    let args = RunArgs {
        command: RunCommands::Diff { run_a, run_b },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("different workflows")
    );
}

#[tokio::test]
async fn run_diff_same_id_fails() {
    let (base_url, token, store) = spawn_server_with_store().await;
    let client = make_client(&base_url, &token);

    let run_a = create_run(&store, "deploy").await;

    let args = RunArgs {
        command: RunCommands::Diff {
            run_a,
            run_b: run_a,
        },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("same"));
}
