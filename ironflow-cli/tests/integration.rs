//! Integration tests: CLI commands against a real ironflow-api server (in-memory store).
//!
//! Reuses the same `spawn_server` pattern as `ironflow-sdk/tests/integration.rs`.

use std::collections::HashMap;
use std::process::Command;
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
use ironflow_sdk::client::{ClientConfig, ListAuditLogsFilter};
use ironflow_sdk::types::{ApiKeyScope, CreateApiKeyRequest, EventKind};
use ironflow_store::crypto::MasterKey;
use ironflow_store::entities::NewUser;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewRun, RunStatus, TriggerKind};
use ironflow_store::store::Store;
use serde_json::json;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use tokio::task::spawn_blocking;
use uuid::Uuid;

use ironflow_cli::commands;
use ironflow_cli::commands::api_key::{ApiKeyArgs, ApiKeyCommands};
use ironflow_cli::commands::audit_log::{AuditLogArgs, AuditLogCommands};
use ironflow_cli::commands::run::{RunArgs, RunCommands};
use ironflow_cli::commands::secret::{SecretArgs, SecretCommands};
use ironflow_cli::commands::user::{UserArgs, UserCommands};
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
    let (base_url, token, _) = spawn_server_with_store().await;
    (base_url, token)
}

/// Same server, plus a handle on the store so a test can seed states the API
/// alone cannot reach (a run parked in `AwaitingApproval` has no worker here).
async fn spawn_server_with_store() -> (String, String, Arc<dyn Store>) {
    let mut memory = InMemoryStore::new();
    // Secrets are encrypted at rest; without a key every secret route fails.
    memory.set_master_key(MasterKey::from_bytes(&[42u8; 32]).unwrap());
    let store: Arc<dyn Store> = Arc::new(memory);
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
    let token = AccessToken::for_user(user.id, "cli-test", true, &jwt_cfg).unwrap();

    (base_url, token.0, store)
}

/// Park a run in `AwaitingApproval`, the only state `run reject` accepts.
async fn seed_awaiting_approval_run(store: &Arc<dyn Store>) -> Uuid {
    let run = store
        .create_run(NewRun {
            created_by: None,
            workflow_name: "deploy".to_string(),
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
    store
        .update_run_status(run.id, RunStatus::AwaitingApproval)
        .await
        .unwrap();

    run.id
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
            created_by: None,
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
            created_by: None,
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
            created_by: None,
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
            idempotency_key: None,
            max_cost: None,
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
            idempotency_key: None,
            max_cost: None,
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
            idempotency_key: None,
            max_cost: None,
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
            idempotency_key: None,
            max_cost: None,
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

// ── Run reject ────────────────────────────────────────────────

#[tokio::test]
async fn run_reject_not_found() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Reject { id: Uuid::now_v7() },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn run_reject_fails_an_awaiting_approval_run() {
    let (base_url, token, store) = spawn_server_with_store().await;
    let client = make_client(&base_url, &token);
    let run_id = seed_awaiting_approval_run(&store).await;

    let args = RunArgs {
        command: RunCommands::Reject { id: run_id },
    };
    commands::run::execute(&client, &args, false, false)
        .await
        .unwrap();

    let run = client.get_run(run_id).await.unwrap();
    assert_eq!(run.data.run.status.to_string(), "failed");
}

#[tokio::test]
async fn run_reject_refuses_a_run_that_is_not_awaiting_approval() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let create = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: None,
            max_retries: None,
            idempotency_key: None,
            max_cost: None,
        },
    };
    commands::run::execute(&client, &create, false, false)
        .await
        .unwrap();
    let run_id = client.list_runs().await.unwrap().data[0].id;

    let args = RunArgs {
        command: RunCommands::Reject { id: run_id },
    };
    assert!(
        commands::run::execute(&client, &args, false, false)
            .await
            .is_err()
    );
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
            idempotency_key: None,
            max_cost: None,
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
            idempotency_key: None,
            max_cost: None,
        },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("cannot read"));
}

// ── Run create with --idempotency-key ─────────────────────────

#[tokio::test]
async fn run_create_with_idempotency_key_creates_one_run() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: Some(r#"{"env": "prod"}"#.to_string()),
            payload_file: None,
            max_retries: None,
            idempotency_key: Some("github:abc-123".to_string()),
            max_cost: None,
        },
    };

    for _ in 0..3 {
        commands::run::execute(&client, &args, false, false)
            .await
            .unwrap();
    }

    let runs = client.list_runs().await.unwrap();
    assert_eq!(runs.data.len(), 1, "the key must collapse the three calls");
    assert_eq!(
        runs.data[0].idempotency_key.as_deref(),
        Some("github:abc-123")
    );
}

#[tokio::test]
async fn run_create_without_idempotency_key_creates_several_runs() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: Some(r#"{"env": "prod"}"#.to_string()),
            payload_file: None,
            max_retries: None,
            idempotency_key: None,
            max_cost: None,
        },
    };

    for _ in 0..3 {
        commands::run::execute(&client, &args, false, false)
            .await
            .unwrap();
    }

    let runs = client.list_runs().await.unwrap();
    assert_eq!(runs.data.len(), 3);
}

#[tokio::test]
async fn run_create_with_a_conflicting_idempotency_key_errors() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let first = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: Some(r#"{"env": "prod"}"#.to_string()),
            payload_file: None,
            max_retries: None,
            idempotency_key: Some("github:abc-123".to_string()),
            max_cost: None,
        },
    };
    commands::run::execute(&client, &first, false, false)
        .await
        .unwrap();

    let conflicting = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: Some(r#"{"env": "staging"}"#.to_string()),
            payload_file: None,
            max_retries: None,
            idempotency_key: Some("github:abc-123".to_string()),
            max_cost: None,
        },
    };
    let result = commands::run::execute(&client, &conflicting, false, false).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn run_create_with_an_empty_idempotency_key_errors() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: None,
            max_retries: None,
            idempotency_key: Some(String::new()),
            max_cost: None,
        },
    };

    assert!(
        commands::run::execute(&client, &args, false, false)
            .await
            .is_err()
    );
}

// ── Cost cap ──────────────────────────────────────────────────

#[tokio::test]
async fn run_create_with_max_cost_reaches_the_api() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: None,
            max_retries: None,
            idempotency_key: None,
            max_cost: Some(2.5),
        },
    };
    commands::run::execute(&client, &args, false, false)
        .await
        .unwrap();

    let runs = client.list_runs().await.unwrap();
    assert_eq!(runs.data[0].max_cost_usd, Some(2.5));
}

#[tokio::test]
async fn run_create_rejects_negative_max_cost_before_calling_the_api() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: None,
            max_retries: None,
            idempotency_key: None,
            max_cost: Some(-1.0),
        },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("zero or positive"));

    // Nothing reached the server.
    assert!(client.list_runs().await.unwrap().data.is_empty());
}

#[tokio::test]
async fn run_create_rejects_non_finite_max_cost() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: None,
            max_retries: None,
            idempotency_key: None,
            max_cost: Some(f64::NAN),
        },
    };
    let result = commands::run::execute(&client, &args, false, false).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("finite"));
}

// ── Secrets ───────────────────────────────────────────────────

#[tokio::test]
async fn secret_list_is_empty_on_a_fresh_server() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = SecretArgs {
        command: SecretCommands::List,
    };
    commands::secret::execute(&client, &args, false)
        .await
        .unwrap();
    commands::secret::execute(&client, &args, true)
        .await
        .unwrap();

    assert!(client.list_secrets().await.unwrap().data.is_empty());
}

#[tokio::test]
async fn secret_set_then_list_exposes_the_key_but_not_the_value() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let set = SecretArgs {
        command: SecretCommands::Set {
            key: "workflows/inbox/gmail_token".to_string(),
            value: Some("s3cr3t-value".to_string()),
        },
    };
    commands::secret::execute(&client, &set, false)
        .await
        .unwrap();

    let secrets = client.list_secrets().await.unwrap();
    assert_eq!(secrets.data.len(), 1);
    assert_eq!(secrets.data[0].key, "workflows/inbox/gmail_token");

    // The value is absent from the API payload itself, table or not.
    let raw = serde_json::to_string(&secrets).unwrap();
    assert!(!raw.contains("s3cr3t-value"), "value leaked in {raw}");
}

#[tokio::test]
async fn secret_set_replaces_an_existing_value_without_duplicating_the_key() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    for value in ["first", "second"] {
        let args = SecretArgs {
            command: SecretCommands::Set {
                key: "db/password".to_string(),
                value: Some(value.to_string()),
            },
        };
        commands::secret::execute(&client, &args, false)
            .await
            .unwrap();
    }

    assert_eq!(client.list_secrets().await.unwrap().data.len(), 1);
}

#[tokio::test]
async fn secret_set_rejects_an_empty_value() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = SecretArgs {
        command: SecretCommands::Set {
            key: "db/password".to_string(),
            value: Some(String::new()),
        },
    };
    let err = commands::secret::execute(&client, &args, false)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("must not be empty"), "{err}");

    // Nothing reached the server.
    assert!(client.list_secrets().await.unwrap().data.is_empty());
}

#[tokio::test]
async fn secret_update_requires_an_existing_key() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = SecretArgs {
        command: SecretCommands::Update {
            key: "never/created".to_string(),
            value: Some("value".to_string()),
        },
    };
    assert!(
        commands::secret::execute(&client, &args, false)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn secret_update_replaces_the_value_of_an_existing_key() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let set = SecretArgs {
        command: SecretCommands::Set {
            key: "db/password".to_string(),
            value: Some("first".to_string()),
        },
    };
    commands::secret::execute(&client, &set, false)
        .await
        .unwrap();

    let update = SecretArgs {
        command: SecretCommands::Update {
            key: "db/password".to_string(),
            value: Some("second".to_string()),
        },
    };
    commands::secret::execute(&client, &update, false)
        .await
        .unwrap();

    let secrets = client.list_secrets().await.unwrap();
    assert_eq!(secrets.data.len(), 1);
    assert!(secrets.data[0].updated_at >= secrets.data[0].created_at);
}

#[tokio::test]
async fn secret_delete_removes_the_key() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let set = SecretArgs {
        command: SecretCommands::Set {
            key: "db/password".to_string(),
            value: Some("value".to_string()),
        },
    };
    commands::secret::execute(&client, &set, false)
        .await
        .unwrap();

    let delete = SecretArgs {
        command: SecretCommands::Delete {
            key: "db/password".to_string(),
            yes: true,
        },
    };
    commands::secret::execute(&client, &delete, false)
        .await
        .unwrap();
    assert!(client.list_secrets().await.unwrap().data.is_empty());
}

#[tokio::test]
async fn secret_delete_without_confirmation_does_not_delete() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let set = SecretArgs {
        command: SecretCommands::Set {
            key: "db/password".to_string(),
            value: Some("value".to_string()),
        },
    };
    commands::secret::execute(&client, &set, false)
        .await
        .unwrap();

    // `cargo test` runs with a piped stdin, so the confirmation must refuse
    // rather than prompt into the void.
    let delete = SecretArgs {
        command: SecretCommands::Delete {
            key: "db/password".to_string(),
            yes: false,
        },
    };
    let err = commands::secret::execute(&client, &delete, false)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("--yes"), "{err}");
    assert_eq!(client.list_secrets().await.unwrap().data.len(), 1);
}

#[tokio::test]
async fn secret_delete_unknown_key_errors() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = SecretArgs {
        command: SecretCommands::Delete {
            key: "never/created".to_string(),
            yes: true,
        },
    };
    assert!(
        commands::secret::execute(&client, &args, false)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn secret_commands_are_refused_to_non_admins() {
    let (base_url, _) = spawn_server().await;
    let client = make_client(&base_url, "invalid-token");

    let args = SecretArgs {
        command: SecretCommands::List,
    };
    assert!(
        commands::secret::execute(&client, &args, false)
            .await
            .is_err()
    );
}

// ── API keys ──────────────────────────────────────────────────

#[tokio::test]
async fn api_key_scopes_lists_the_grantable_scopes() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = ApiKeyArgs {
        command: ApiKeyCommands::Scopes,
    };
    commands::api_key::execute(&client, &args, false)
        .await
        .unwrap();

    assert!(!client.available_scopes().await.unwrap().data.is_empty());
}

#[tokio::test]
async fn api_key_create_then_list_then_delete() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let create = ApiKeyArgs {
        command: ApiKeyCommands::Create {
            name: "ci-deploy".to_string(),
            scopes: vec![ApiKeyScope::RunsRead, ApiKeyScope::RunsWrite],
            expires_at: None,
        },
    };
    commands::api_key::execute(&client, &create, false)
        .await
        .unwrap();

    let keys = client.list_api_keys().await.unwrap();
    assert_eq!(keys.data.len(), 1);
    assert_eq!(keys.data[0].name, "ci-deploy");
    assert_eq!(keys.data[0].scopes.len(), 2);

    let list = ApiKeyArgs {
        command: ApiKeyCommands::List,
    };
    commands::api_key::execute(&client, &list, false)
        .await
        .unwrap();
    commands::api_key::execute(&client, &list, true)
        .await
        .unwrap();

    let delete = ApiKeyArgs {
        command: ApiKeyCommands::Delete {
            id: keys.data[0].id,
            yes: true,
        },
    };
    commands::api_key::execute(&client, &delete, false)
        .await
        .unwrap();
    assert!(client.list_api_keys().await.unwrap().data.is_empty());
}

#[tokio::test]
async fn api_key_list_never_exposes_a_raw_key() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let create = ApiKeyArgs {
        command: ApiKeyCommands::Create {
            name: "ci-deploy".to_string(),
            scopes: vec![ApiKeyScope::RunsRead],
            expires_at: None,
        },
    };
    commands::api_key::execute(&client, &create, false)
        .await
        .unwrap();

    let created = client
        .create_api_key(
            &CreateApiKeyRequest::builder()
                .name("second".to_string())
                .scopes(vec![ApiKeyScope::RunsRead])
                .expires_at(None)
                .try_into()
                .unwrap(),
        )
        .await
        .unwrap();

    let listed = serde_json::to_string(&client.list_api_keys().await.unwrap()).unwrap();
    assert!(
        !listed.contains(&created.data.key),
        "raw key leaked in {listed}"
    );
}

#[tokio::test]
async fn api_key_delete_without_confirmation_does_not_delete() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let create = ApiKeyArgs {
        command: ApiKeyCommands::Create {
            name: "ci-deploy".to_string(),
            scopes: vec![ApiKeyScope::RunsRead],
            expires_at: None,
        },
    };
    commands::api_key::execute(&client, &create, false)
        .await
        .unwrap();
    let id = client.list_api_keys().await.unwrap().data[0].id;

    let delete = ApiKeyArgs {
        command: ApiKeyCommands::Delete { id, yes: false },
    };
    assert!(
        commands::api_key::execute(&client, &delete, false)
            .await
            .is_err()
    );
    assert_eq!(client.list_api_keys().await.unwrap().data.len(), 1);
}

#[tokio::test]
async fn api_key_delete_unknown_id_errors() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = ApiKeyArgs {
        command: ApiKeyCommands::Delete {
            id: Uuid::now_v7(),
            yes: true,
        },
    };
    assert!(
        commands::api_key::execute(&client, &args, false)
            .await
            .is_err()
    );
}

// ── Users ─────────────────────────────────────────────────────

#[tokio::test]
async fn user_list_contains_the_seeded_admin() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = UserArgs {
        command: UserCommands::List,
    };
    commands::user::execute(&client, &args, false)
        .await
        .unwrap();
    commands::user::execute(&client, &args, true).await.unwrap();

    let users = client.list_users().await.unwrap();
    assert!(users.data.iter().any(|u| u.username == "cli-test"));
}

#[tokio::test]
async fn user_create_then_set_role_then_delete() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let create = UserArgs {
        command: UserCommands::Create {
            username: "alice".to_string(),
            email: "alice@test.local".to_string(),
            password: Some("hunter2hunter2".to_string()),
            admin: false,
        },
    };
    commands::user::execute(&client, &create, false)
        .await
        .unwrap();

    let alice = client
        .list_users()
        .await
        .unwrap()
        .data
        .into_iter()
        .find(|u| u.username == "alice")
        .expect("alice must exist");
    assert!(!alice.is_admin);

    let promote = UserArgs {
        command: UserCommands::SetRole {
            id: alice.id,
            admin: true,
            member: false,
        },
    };
    commands::user::execute(&client, &promote, false)
        .await
        .unwrap();
    assert!(
        client
            .list_users()
            .await
            .unwrap()
            .data
            .iter()
            .find(|u| u.id == alice.id)
            .unwrap()
            .is_admin
    );

    let demote = UserArgs {
        command: UserCommands::SetRole {
            id: alice.id,
            admin: false,
            member: true,
        },
    };
    commands::user::execute(&client, &demote, false)
        .await
        .unwrap();
    assert!(
        !client
            .list_users()
            .await
            .unwrap()
            .data
            .iter()
            .find(|u| u.id == alice.id)
            .unwrap()
            .is_admin
    );

    let delete = UserArgs {
        command: UserCommands::Delete {
            id: alice.id,
            yes: true,
        },
    };
    commands::user::execute(&client, &delete, false)
        .await
        .unwrap();
    assert!(
        !client
            .list_users()
            .await
            .unwrap()
            .data
            .iter()
            .any(|u| u.id == alice.id)
    );
}

#[tokio::test]
async fn user_create_rejects_an_empty_password() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = UserArgs {
        command: UserCommands::Create {
            username: "alice".to_string(),
            email: "alice@test.local".to_string(),
            password: Some(String::new()),
            admin: false,
        },
    };
    let err = commands::user::execute(&client, &args, false)
        .await
        .unwrap_err();
    assert!(err.to_string().contains("must not be empty"), "{err}");
}

#[tokio::test]
async fn user_create_rejects_a_duplicate_email() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = UserArgs {
        command: UserCommands::Create {
            username: "clone".to_string(),
            email: "cli-test@test.local".to_string(),
            password: Some("hunter2hunter2".to_string()),
            admin: false,
        },
    };
    assert!(
        commands::user::execute(&client, &args, false)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn user_delete_without_confirmation_does_not_delete() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let id = client.list_users().await.unwrap().data[0].id;
    let args = UserArgs {
        command: UserCommands::Delete { id, yes: false },
    };
    assert!(
        commands::user::execute(&client, &args, false)
            .await
            .is_err()
    );
    assert!(!client.list_users().await.unwrap().data.is_empty());
}

#[tokio::test]
async fn user_set_role_unknown_id_errors() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = UserArgs {
        command: UserCommands::SetRole {
            id: Uuid::now_v7(),
            admin: true,
            member: false,
        },
    };
    assert!(
        commands::user::execute(&client, &args, false)
            .await
            .is_err()
    );
}

// ── Audit logs ────────────────────────────────────────────────

/// `audit-log list` with the given filters, both in table and JSON mode.
fn audit_log_list(run: Option<Uuid>, event_type: Option<EventKind>) -> AuditLogArgs {
    AuditLogArgs {
        command: AuditLogCommands::List {
            run,
            event_type,
            from: None,
            to: None,
            page: None,
            per_page: None,
        },
    }
}

#[tokio::test]
async fn audit_log_list_without_filters() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = audit_log_list(None, None);
    commands::audit_log::execute(&client, &args, false)
        .await
        .unwrap();
    commands::audit_log::execute(&client, &args, true)
        .await
        .unwrap();
}

#[tokio::test]
async fn audit_log_list_filters_by_run() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let create = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: None,
            max_retries: None,
            idempotency_key: None,
            max_cost: None,
        },
    };
    commands::run::execute(&client, &create, false, false)
        .await
        .unwrap();
    let run_id = client.list_runs().await.unwrap().data[0].id;

    let args = audit_log_list(Some(run_id), None);
    commands::audit_log::execute(&client, &args, false)
        .await
        .unwrap();

    let unknown = ListAuditLogsFilter {
        run_id: Some(Uuid::now_v7()),
        ..Default::default()
    };
    assert!(
        client
            .list_audit_logs_filtered(&unknown)
            .await
            .unwrap()
            .data
            .is_empty()
    );
}

#[tokio::test]
async fn audit_log_list_filters_by_event_type() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = audit_log_list(None, Some(EventKind::RunCreated));
    commands::audit_log::execute(&client, &args, false)
        .await
        .unwrap();

    let filter = ListAuditLogsFilter {
        event_type: Some("user_signed_out"),
        ..Default::default()
    };
    let entries = client.list_audit_logs_filtered(&filter).await.unwrap();
    assert!(
        entries
            .data
            .iter()
            .all(|e| e.event_type == EventKind::UserSignedOut)
    );
}

#[tokio::test]
async fn audit_log_list_is_refused_without_a_valid_token() {
    let (base_url, _) = spawn_server().await;
    let client = make_client(&base_url, "invalid-token");

    let args = audit_log_list(None, None);
    assert!(
        commands::audit_log::execute(&client, &args, false)
            .await
            .is_err()
    );
}

// ── End-to-end: the real binary must never print a secret value ───

/// Run the compiled CLI against the test server and return `stdout + stderr`.
///
/// The subprocess is waited on in a blocking pool thread: waiting inline would
/// park the runtime thread that also drives the test server, and the request
/// would time out against a server that never gets polled.
async fn run_binary(base_url: &str, token: &str, args: &[&str]) -> String {
    let base_url = base_url.to_string();
    let token = token.to_string();
    let args: Vec<String> = args.iter().map(|a| (*a).to_string()).collect();

    spawn_blocking(move || {
        let output = Command::new(env!("CARGO_BIN_EXE_ironflow-cli"))
            .args(["--url", &base_url, "--api-key", &token])
            .args(&args)
            .output()
            .expect("failed to run the ironflow-cli binary");

        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
    .await
    .expect("the ironflow-cli subprocess panicked")
}

#[tokio::test]
async fn the_binary_never_prints_a_secret_value() {
    let (base_url, token) = spawn_server().await;
    const VALUE: &str = "s3cr3t-canary-value";

    let set = run_binary(&base_url, &token, &["secret", "set", "db/password", VALUE]).await;
    assert!(!set.contains(VALUE), "value leaked by `secret set`: {set}");

    for mode in [vec!["secret", "list"], vec!["--json", "secret", "list"]] {
        let listed = run_binary(&base_url, &token, &mode).await;
        assert!(
            listed.contains("db/password"),
            "key missing from `{mode:?}`: {listed}"
        );
        assert!(
            !listed.contains(VALUE),
            "value leaked by `{mode:?}`: {listed}"
        );
    }

    let updated = run_binary(
        &base_url,
        &token,
        &[
            "--json",
            "secret",
            "update",
            "db/password",
            "rotated-canary",
        ],
    )
    .await;
    assert!(
        !updated.contains("rotated-canary"),
        "value leaked by `secret update`: {updated}"
    );
}

#[tokio::test]
async fn the_binary_refuses_to_delete_without_a_confirmation() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    run_binary(&base_url, &token, &["secret", "set", "db/password", "v"]).await;

    let output = run_binary(&base_url, &token, &["secret", "delete", "db/password"]).await;
    assert!(output.contains("--yes"), "unexpected output: {output}");
    assert_eq!(client.list_secrets().await.unwrap().data.len(), 1);

    run_binary(
        &base_url,
        &token,
        &["secret", "delete", "db/password", "--yes"],
    )
    .await;
    assert!(client.list_secrets().await.unwrap().data.is_empty());
}

#[tokio::test]
async fn the_binary_shows_a_created_api_key_once() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let created = run_binary(
        &base_url,
        &token,
        &["api-key", "create", "ci", "--scope", "runs_read"],
    )
    .await;
    assert!(
        created.contains("only time the key is shown"),
        "missing warning: {created}"
    );

    let raw_key = client
        .list_api_keys()
        .await
        .unwrap()
        .data
        .first()
        .map(|k| k.key_prefix.clone())
        .expect("the key must exist");
    assert!(created.contains(&raw_key), "prefix missing: {created}");

    let listed = run_binary(&base_url, &token, &["api-key", "list"]).await;
    assert!(
        !listed.contains("only time the key is shown"),
        "`list` must not warn: {listed}"
    );
}

#[tokio::test]
async fn run_create_accepts_zero_max_cost() {
    let (base_url, token) = spawn_server().await;
    let client = make_client(&base_url, &token);

    let args = RunArgs {
        command: RunCommands::Create {
            workflow: "deploy".to_string(),
            payload: None,
            payload_file: None,
            max_retries: None,
            idempotency_key: None,
            max_cost: Some(0.0),
        },
    };
    commands::run::execute(&client, &args, false, false)
        .await
        .unwrap();

    let runs = client.list_runs().await.unwrap();
    assert_eq!(runs.data[0].max_cost_usd, Some(0.0));
}
