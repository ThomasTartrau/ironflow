//! Tests for artifact download methods.

use std::sync::Arc;
use std::time::Duration;

use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_artifacts::blob_store::BlobStore;
use ironflow_artifacts::local::LocalBlobStore;
use ironflow_artifacts::stream_from_bytes;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_auth::password;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_sdk::IronflowClient;
use ironflow_sdk::client::ClientConfig;
use ironflow_store::entities::{NewUser, RunActor};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{NewArtifact, NewStep, StepKind, step_trace_id};
use ironflow_store::store::Store;
use tempfile::TempDir;
use tokio::net::TcpListener;
use tokio::sync::broadcast;
use uuid::Uuid;

struct TestWorkflow;

impl WorkflowHandler for TestWorkflow {
    fn name(&self) -> &str {
        "test-wf"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

fn jwt_config() -> Arc<JwtConfig> {
    Arc::new(JwtConfig {
        secret: "artifact-test-secret".to_string(),
        access_token_ttl_secs: 900,
        refresh_token_ttl_secs: 604800,
        cookie_domain: None,
        cookie_secure: false,
    })
}

struct Fixture {
    base_url: String,
    token: String,
    run_id: Uuid,
    step_id: Uuid,
    _dir: TempDir,
}

async fn spawn_server_with_artifact() -> Fixture {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);
    engine.register(TestWorkflow).unwrap();

    let jwt_cfg = jwt_config();
    let (event_sender, _) = broadcast::channel::<Event>(16);

    let hash = password::hash("test-password").unwrap();
    let user = store
        .create_user(NewUser {
            email: "artifact-test@test.local".to_string(),
            username: "artifact-test".to_string(),
            password_hash: hash,
            is_admin: Some(true),
        })
        .await
        .unwrap();

    let dir = TempDir::new().unwrap();
    let blob: Arc<dyn BlobStore> = Arc::new(LocalBlobStore::new(dir.path()));

    let run = store
        .create_run(ironflow_store::models::NewRun {
            created_by: Some(RunActor::User { user_id: user.id }),
            workflow_name: "test-wf".to_string(),
            trigger: ironflow_store::models::TriggerKind::Manual,
            payload: serde_json::json!({}),
            max_retries: 0,
            handler_version: None,
            labels: std::collections::HashMap::new(),
            scheduled_at: None,
            idempotency_key: None,
            max_cost_usd: None,
        })
        .await
        .unwrap()
        .into_run();

    let step = store
        .create_step(NewStep {
            run_id: run.id,
            trace_id: step_trace_id(run.id, "build", 0),
            name: "build".to_string(),
            kind: StepKind::Shell,
            position: 0,
            input: None,
            is_error_handler: false,
        })
        .await
        .unwrap();

    let artifact_id = Uuid::now_v7();
    let key = format!("artifacts/{}/{}/{artifact_id}", run.id, step.id);
    let content = b"<html>test artifact</html>";
    let digest = blob
        .put(&key, stream_from_bytes(content.to_vec()))
        .await
        .unwrap();

    store
        .create_artifact(NewArtifact {
            id: artifact_id,
            run_id: run.id,
            step_id: step.id,
            name: "report.html".to_string(),
            storage_key: key,
            content_type: "text/html".to_string(),
            size_bytes: digest.size_bytes,
            sha256: digest.sha256,
        })
        .await
        .unwrap();

    let state = AppState::new(
        store,
        Arc::new(engine),
        jwt_cfg.clone(),
        "test-worker-token".to_string(),
        event_sender,
    )
    .with_blob_store(blob);

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
    let token = AccessToken::for_user(user.id, "artifact-test", true, &jwt_cfg).unwrap();

    Fixture {
        base_url,
        token: token.0,
        run_id: run.id,
        step_id: step.id,
        _dir: dir,
    }
}

fn make_client(base_url: &str, token: &str) -> IronflowClient {
    let config = ClientConfig {
        base_url: base_url.to_string(),
        api_key: token.to_string(),
        timeout: Duration::from_secs(10),
    };
    IronflowClient::from_config(config)
}

// ── Row 6: download artifact by step_name ────────────────────────

#[tokio::test]
async fn download_artifact_by_name() {
    let fixture = spawn_server_with_artifact().await;
    let client = make_client(&fixture.base_url, &fixture.token);

    let download = client
        .download_artifact(fixture.run_id, "build", "report.html")
        .await
        .unwrap();

    assert_eq!(download.content_type, "text/html");
    assert_eq!(&download.bytes[..], b"<html>test artifact</html>");
    assert!(
        !download.sha256.is_empty(),
        "sha256 should be populated from step metadata"
    );
}

// ── Row 7: download artifact unknown step returns error ──────────

#[tokio::test]
async fn download_artifact_unknown_step() {
    let fixture = spawn_server_with_artifact().await;
    let client = make_client(&fixture.base_url, &fixture.token);

    let err = client
        .download_artifact(fixture.run_id, "nonexistent-step", "report.html")
        .await
        .unwrap_err();

    assert!(
        format!("{err}").contains("not found"),
        "error should mention step not found: {err}"
    );
}

// ── Row 8: download artifact by step_id ──────────────────────────

#[tokio::test]
async fn download_artifact_by_step_id() {
    let fixture = spawn_server_with_artifact().await;
    let client = make_client(&fixture.base_url, &fixture.token);

    let download = client
        .download_artifact_by_step_id(fixture.run_id, fixture.step_id, "report.html")
        .await
        .unwrap();

    assert_eq!(download.content_type, "text/html");
    assert_eq!(&download.bytes[..], b"<html>test artifact</html>");
}
