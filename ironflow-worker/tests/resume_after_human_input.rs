//! Split API + worker deployment: a human input answered through the API
//! requeues the run, and a worker finishes it without re-running the steps
//! that completed before the input.
//!
//! Everything is real: the router built by `create_router` served over TCP, an
//! `InMemoryStore`, an `Engine` in `ExecutionMode::Workers` on the API side and
//! a `Worker` polling that API.

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::serve;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::{AccessToken, JwtConfig};
use ironflow_core::error::OperationError;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::{HumanInputConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_engine::operation::{Operation, OperationContext};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepKind, StepStatus, TriggerKind};
use ironflow_store::store::RunStore;
use ironflow_worker::WorkerBuilder;
use reqwest::{Client, StatusCode};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Workflow name registered on both the API and the worker.
const WORKFLOW: &str = "resume-after-input";

/// Wall-clock budget for the whole flow.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Deserialize, JsonSchema)]
struct Answers {
    answers: Vec<String>,
}

/// Runs a step, asks for [`Answers`] and records them.
struct ResumeAfterInput {
    seen: Arc<Mutex<Vec<String>>>,
}

impl WorkflowHandler for ResumeAfterInput {
    fn name(&self) -> &str {
        WORKFLOW
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("before-input", ShellConfig::new("true")).await?;
            let answers: Answers = ctx
                .human_input("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            self.seen.lock().expect("seen lock").extend(answers.answers);
            Ok(())
        })
    }
}

#[tokio::test]
async fn worker_resumes_run_after_human_input_without_rerunning_prior_steps() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let seen = Arc::new(Mutex::new(Vec::new()));

        let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()))
            .with_execution_mode(ExecutionMode::Workers);
        engine
            .register(ResumeAfterInput { seen: seen.clone() })
            .expect("register handler");
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-resume-after-input".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(16);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config.clone(),
            "test-worker-token".to_string(),
            event_sender,
        );

        // The first execution always runs locally and suspends on the input.
        let result = state
            .engine
            .run_handler(WORKFLOW, TriggerKind::Manual, json!({}))
            .await
            .expect("run suspends");
        let run_id = result.run.id;
        assert_eq!(result.run.status.state, RunStatus::AwaitingApproval);

        let steps = store.list_steps(run_id).await.expect("list steps");
        assert_eq!(steps.iter().filter(|s| s.name == "before-input").count(), 1);
        let step_id = steps
            .iter()
            .find(|s| s.kind == StepKind::HumanInput)
            .expect("human input step")
            .id;

        // The limiter keys on the peer address, which `axum::serve` without
        // connect info does not provide.
        let config = RouterConfig {
            rate_limit_auth: None,
            rate_limit_general: None,
            ..RouterConfig::default()
        };
        let router = create_router(state.clone(), config);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        spawn(async move {
            serve(listener, router).await.expect("serve");
        });
        let base_url = format!("http://{addr}");

        let worker = WorkerBuilder::new(&base_url, "test-worker-token")
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .register(ResumeAfterInput { seen: seen.clone() })
            .worker_id("worker-resume-after-input")
            .concurrency(1)
            .poll_interval(Duration::from_millis(20))
            .lease_ttl(Duration::from_secs(5))
            .lease_refresh_interval(Duration::from_millis(500))
            .run_timeout(Duration::from_secs(10))
            .build()
            .expect("build worker");
        let handle = spawn(async move {
            if let Err(e) = worker.run().await {
                eprintln!("worker exited with error: {e:?}");
            }
        });

        let token =
            AccessToken::for_user(Uuid::now_v7(), "admin", true, &jwt_config).expect("token");
        let resp = Client::new()
            .post(format!(
                "{base_url}/api/v1/runs/{run_id}/steps/{step_id}/input"
            ))
            .bearer_auth(&token.0)
            .json(&json!({"answers": ["ok"]}))
            .send()
            .await
            .expect("post answer");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.expect("json body");
        // In workers mode the API requeues the run instead of resuming it.
        assert_eq!(body["data"]["status"], "pending");

        // A generous deadline (not a fixed sleep) keeps the test reliable on a
        // slow CI executor, where the worker task may be scheduled late.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut status = RunStatus::Pending;
        while Instant::now() < deadline {
            status = store
                .get_run(run_id)
                .await
                .expect("get run")
                .expect("run exists")
                .status
                .state;
            if status == RunStatus::Completed {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        handle.abort();
        assert_eq!(status, RunStatus::Completed);

        let steps = store.list_steps(run_id).await.expect("list steps");
        let before: Vec<_> = steps.iter().filter(|s| s.name == "before-input").collect();
        assert_eq!(before.len(), 1, "the step before the input ran again");
        assert_eq!(before[0].status.state, StepStatus::Completed);
        assert_eq!(
            steps
                .iter()
                .filter(|s| s.kind == StepKind::HumanInput)
                .count(),
            1
        );
        assert_eq!(
            seen.lock().expect("seen lock").clone(),
            vec!["ok".to_string()]
        );
    })
    .await
    .expect("test timed out");
}

/// Workflow name for the operation + parallel + human input regression test.
const WORKFLOW_OP_PARALLEL: &str = "resume-after-input-op-parallel";

/// A custom operation that counts how many times it actually ran.
struct CountingOp {
    calls: Arc<AtomicU32>,
}

#[async_trait]
impl Operation for CountingOp {
    fn kind(&self) -> &str {
        "counting-op"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(json!({"called": true}))
    }
}

/// Runs a custom operation, then a parallel wave, then asks for [`Answers`].
struct OperationThenParallelThenInput {
    op: CountingOp,
    seen: Arc<Mutex<Vec<String>>>,
}

impl WorkflowHandler for OperationThenParallelThenInput {
    fn name(&self) -> &str {
        WORKFLOW_OP_PARALLEL
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.operation("op", &self.op).await?;
            ctx.parallel(
                vec![
                    ("wave-a", StepConfig::Shell(ShellConfig::new("echo a"))),
                    ("wave-b", StepConfig::Shell(ShellConfig::new("echo b"))),
                ],
                true,
            )
            .await?;
            let answers: Answers = ctx
                .human_input("clarify", HumanInputConfig::new("Answer?"))
                .await?;
            self.seen.lock().expect("seen lock").extend(answers.answers);
            Ok(())
        })
    }
}

#[tokio::test]
async fn worker_resumes_run_after_human_input_without_rerunning_operation_or_parallel_steps() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let calls = Arc::new(AtomicU32::new(0));

        let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()))
            .with_execution_mode(ExecutionMode::Workers);
        engine
            .register(OperationThenParallelThenInput {
                op: CountingOp {
                    calls: calls.clone(),
                },
                seen: seen.clone(),
            })
            .expect("register handler");
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-resume-after-input-op-parallel".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(16);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config.clone(),
            "test-worker-token".to_string(),
            event_sender,
        );

        // The first execution always runs locally and suspends on the input.
        let result = state
            .engine
            .run_handler(WORKFLOW_OP_PARALLEL, TriggerKind::Manual, json!({}))
            .await
            .expect("run suspends");
        let run_id = result.run.id;
        assert_eq!(result.run.status.state, RunStatus::AwaitingApproval);

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let steps = store.list_steps(run_id).await.expect("list steps");
        assert_eq!(steps.iter().filter(|s| s.name == "op").count(), 1);
        let op_step_id = steps.iter().find(|s| s.name == "op").expect("op step").id;
        assert_eq!(steps.iter().filter(|s| s.name == "wave-a").count(), 1);
        assert_eq!(steps.iter().filter(|s| s.name == "wave-b").count(), 1);
        let wave_a_id = steps
            .iter()
            .find(|s| s.name == "wave-a")
            .expect("wave-a step")
            .id;
        let wave_b_id = steps
            .iter()
            .find(|s| s.name == "wave-b")
            .expect("wave-b step")
            .id;
        let step_id = steps
            .iter()
            .find(|s| s.kind == StepKind::HumanInput)
            .expect("human input step")
            .id;

        // The limiter keys on the peer address, which `axum::serve` without
        // connect info does not provide.
        let config = RouterConfig {
            rate_limit_auth: None,
            rate_limit_general: None,
            ..RouterConfig::default()
        };
        let router = create_router(state.clone(), config);
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("local addr");
        spawn(async move {
            serve(listener, router).await.expect("serve");
        });
        let base_url = format!("http://{addr}");

        let worker = WorkerBuilder::new(&base_url, "test-worker-token")
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .register(OperationThenParallelThenInput {
                op: CountingOp {
                    calls: calls.clone(),
                },
                seen: seen.clone(),
            })
            .worker_id("worker-resume-after-input-op-parallel")
            .concurrency(1)
            .poll_interval(Duration::from_millis(20))
            .lease_ttl(Duration::from_secs(5))
            .lease_refresh_interval(Duration::from_millis(500))
            .run_timeout(Duration::from_secs(10))
            .build()
            .expect("build worker");
        let handle = spawn(async move {
            if let Err(e) = worker.run().await {
                eprintln!("worker exited with error: {e:?}");
            }
        });

        let token =
            AccessToken::for_user(Uuid::now_v7(), "admin", true, &jwt_config).expect("token");
        let resp = Client::new()
            .post(format!(
                "{base_url}/api/v1/runs/{run_id}/steps/{step_id}/input"
            ))
            .bearer_auth(&token.0)
            .json(&json!({"answers": ["ok"]}))
            .send()
            .await
            .expect("post answer");
        assert_eq!(resp.status(), StatusCode::OK);
        let body: Value = resp.json().await.expect("json body");
        // In workers mode the API requeues the run instead of resuming it.
        assert_eq!(body["data"]["status"], "pending");

        // A generous deadline (not a fixed sleep) keeps the test reliable on a
        // slow CI executor, where the worker task may be scheduled late.
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut status = RunStatus::Pending;
        while Instant::now() < deadline {
            status = store
                .get_run(run_id)
                .await
                .expect("get run")
                .expect("run exists")
                .status
                .state;
            if status == RunStatus::Completed {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
        handle.abort();
        assert_eq!(status, RunStatus::Completed);

        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the operation must not be re-executed on resume"
        );

        let steps = store.list_steps(run_id).await.expect("list steps");
        let op_steps: Vec<_> = steps.iter().filter(|s| s.name == "op").collect();
        assert_eq!(op_steps.len(), 1, "no second 'op' step must be created");
        assert_eq!(op_steps[0].id, op_step_id);

        let wave_a_steps: Vec<_> = steps.iter().filter(|s| s.name == "wave-a").collect();
        let wave_b_steps: Vec<_> = steps.iter().filter(|s| s.name == "wave-b").collect();
        assert_eq!(
            wave_a_steps.len(),
            1,
            "no second 'wave-a' step must be created"
        );
        assert_eq!(
            wave_b_steps.len(),
            1,
            "no second 'wave-b' step must be created"
        );
        assert_eq!(wave_a_steps[0].id, wave_a_id);
        assert_eq!(wave_b_steps[0].id, wave_b_id);

        assert_eq!(
            steps
                .iter()
                .filter(|s| s.kind == StepKind::HumanInput)
                .count(),
            1
        );
        assert_eq!(
            seen.lock().expect("seen lock").clone(),
            vec!["ok".to_string()]
        );
    })
    .await
    .expect("test timed out");
}
