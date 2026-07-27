//! Integration tests: CLI commands against a real ironflow-api server (in-memory store).
//!
//! Reuses the same `spawn_server` pattern as `ironflow-sdk/tests/integration.rs`.

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

use ironflow_cli::commands;
use ironflow_cli::commands::run::{RunArgs, RunCommands};
use ironflow_cli::commands::workflow::{WorkflowArgs, WorkflowCommands};

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
        secret: "cli-integration-test-secret".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    })
}

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
            email: "cli-test@test.local".to_string(),
            username: "cli-test".to_string(),
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
    let token = AccessToken::for_user(user.id, "cli-test", true, &jwt_cfg).unwrap();

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

// ── Stats ──────────────────────────────────────────────────────

#[tokio::test]
async fn stats_table_output() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    commands::stats::execute(&client, false).await.unwrap();
}

#[tokio::test]
async fn stats_json_output() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    commands::stats::execute(&client, true).await.unwrap();
}

// ── Workflow ───────────────────────────────────────────────────

#[tokio::test]
async fn workflow_list_table() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = WorkflowArgs {
        command: WorkflowCommands::List,
    };
    commands::workflow::execute(&client, &args, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn workflow_list_json() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = WorkflowArgs {
        command: WorkflowCommands::List,
    };
    commands::workflow::execute(&client, &args, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn workflow_get_existing() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = WorkflowArgs {
        command: WorkflowCommands::Get {
            name: "deploy".to_string(),
        },
    };
    commands::workflow::execute(&client, &args, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn workflow_get_not_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = WorkflowArgs {
        command: WorkflowCommands::Get {
            name: "nonexistent".to_string(),
        },
    };
    let result = commands::workflow::execute(&client, &args, false).await;
    assert!(result.is_err());
}

// ── Run list ──────────────────────────────────────────────────

#[tokio::test]
async fn run_list_empty_table() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::List {
            status: None,
            workflow: None,
            page: None,
            per_page: None,
        },
    };
    commands::run::execute(&client, &args, false, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_list_empty_json() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::List {
            status: None,
            workflow: None,
            page: None,
            per_page: None,
        },
    };
    commands::run::execute(&client, &args, true, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_list_with_filters() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::List {
            status: Some("completed".to_string()),
            workflow: Some("deploy".to_string()),
            page: Some(1),
            per_page: Some(10),
        },
    };
    commands::run::execute(&client, &args, false, false)
        .await
        .unwrap();
}

// ── Run create ────────────────────────────────────────────────

#[tokio::test]
async fn run_create_and_get() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: Some(r#"{"env": "staging"}"#.to_string()),
            payload_file: None,
            max_retries: None,
        },
    };
    commands::run::execute(&client, &args, false, false)
        .await
        .unwrap();

    let runs = client.list_runs().await.unwrap();
    assert_eq!(runs.data.len(), 1);
    let run_id = runs.data[0].id;

    let get_args = RunArgs {
        command: RunCommands::Get { id: run_id },
    };
    commands::run::execute(&client, &get_args, false, false)
        .await
        .unwrap();
    commands::run::execute(&client, &get_args, true, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_create_unknown_workflow() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "nonexistent".to_string(),
            payload: None,
            payload_file: None,
            max_retries: None,
        },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn run_create_invalid_payload() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: Some("not valid json".to_string()),
            payload_file: None,
            max_retries: None,
        },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("invalid JSON"));
}

#[tokio::test]
async fn run_create_non_object_payload() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: Some(r#""just a string""#.to_string()),
            payload_file: None,
            max_retries: None,
        },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("JSON object"));
}

// ── Run get not found ─────────────────────────────────────────

#[tokio::test]
async fn run_get_not_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Get { id: Uuid::now_v7() },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
}

// ── Run cancel ────────────────────────────────────────────────

#[tokio::test]
async fn run_cancel_not_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Cancel { id: Uuid::now_v7() },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
}

// ── Run approve ───────────────────────────────────────────────

#[tokio::test]
async fn run_approve_not_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Approve { id: Uuid::now_v7() },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
}

// ── Run retry ─────────────────────────────────────────────────

#[tokio::test]
async fn run_retry_not_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Retry { id: Uuid::now_v7() },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
}

// ── Unauthorized ──────────────────────────────────────────────

#[tokio::test]
async fn unauthorized_returns_error() {
    let (base_url, _) = spawn_server().await;
    let client = make_client(&base_url, "invalid-token");

    let result = commands::stats::execute(&client, false).await;
    assert!(result.is_err());
}

// ── Payload from file ─────────────────────────────────────────

#[tokio::test]
async fn run_create_from_payload_file() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let mut tmp = tempfile::NamedTempFile::new().unwrap();
    std::io::Write::write_all(&mut tmp, br#"{"env": "prod"}"#).unwrap();

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: Some(tmp.path().to_path_buf()),
            max_retries: None,
        },
    };
    commands::run::execute(&client, &args, false, false)
        .await
        .unwrap();
}

#[tokio::test]
async fn run_create_from_missing_file() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: Some("/nonexistent/payload.json".into()),
            max_retries: None,
        },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("cannot read"));
}
