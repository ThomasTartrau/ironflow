//! Split API + worker deployment: a workflow that declares a guard runs on a
//! remote worker (#244).
//!
//! The guard seeds its token budget with the tokens already spent by the run
//! tree, which the engine lists by label. The worker has no store of its own:
//! it lists those runs through the API's internal routes. Before the fix the
//! worker store refused `list_runs`, the run failed before its first step and
//! only showed "worker lease expired".
//!
//! The first tests serve the real router built by `create_router` over TCP,
//! on an `InMemoryStore`, with a `Worker` polling it. The last one uses the
//! stub API of `helpers`, which has no run search, to make the guard fail
//! before the first step.
//!
//! Test names contain `guard_on_worker` so `cargo test -p ironflow-worker
//! guard_on_worker` selects the whole suite.

mod helpers;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::serve;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::provider::LABEL_ROOT_RUN_ID;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::guard::WorkflowGuardConfig;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    NewRun, NewStep, Run, RunFilter, RunStatus, StepKind, StepUpdate, TriggerKind, step_trace_id,
};
use ironflow_store::store::RunStore;
use ironflow_worker::{ApiRunStore, WorkerBuilder};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use helpers::{TestApiState, make_run_json, spawn_test_api};

/// Workflow name of [`GuardedRoot`].
const ROOT: &str = "guarded-root";

/// Workflow name of [`Child`].
const CHILD: &str = "guarded-child";

/// Token budget of [`GuardedRoot`]'s guard.
const TOKEN_BUDGET: u64 = 1_000;

/// Bearer token shared by the API and the worker.
const WORKER_TOKEN: &str = "test-worker-token";

/// Wall-clock budget for a whole test.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline of every polling loop, generous for a slow CI executor.
const POLL_DEADLINE: Duration = Duration::from_secs(20);

/// Payload of a workflow that takes nothing.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoInput {}

/// A sub-workflow with a single shell step.
struct Child;

impl WorkflowHandler for Child {
    fn name(&self) -> &str {
        CHILD
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("child-echo", ShellConfig::new("echo child"))
                .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Child {
    type Input = NoInput;
}

/// A shell step, then [`Child`], under a token budget: entering the child
/// checks the tokens the run tree already spent.
struct GuardedRoot;

impl WorkflowHandler for GuardedRoot {
    fn name(&self) -> &str {
        ROOT
    }

    fn guard_config(&self) -> Option<WorkflowGuardConfig> {
        Some(WorkflowGuardConfig::new().with_max_workflow_tokens(TOKEN_BUDGET))
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("root-echo", ShellConfig::new("echo root"))
                .await?;
            ctx.workflow(&Child, NoInput {}).await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for GuardedRoot {
    type Input = NoInput;
}

/// The real API router served over TCP; the worker is started separately so
/// a test can seed the store before it polls.
struct Api {
    state: AppState,
    store: Arc<InMemoryStore>,
    base_url: String,
}

impl Api {
    async fn start() -> Self {
        let store = Arc::new(InMemoryStore::new());
        let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()))
            .with_execution_mode(ExecutionMode::Workers);
        engine.register(Child).expect("register child");
        engine.register(GuardedRoot).expect("register root");
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-guard-on-worker".to_string(),
            access_token_ttl_secs: 900,
            refresh_token_ttl_secs: 604800,
            cookie_domain: None,
            cookie_secure: false,
        });
        let (event_sender, _) = broadcast::channel::<Event>(16);
        let state = AppState::new(
            store.clone(),
            Arc::new(engine),
            jwt_config,
            WORKER_TOKEN.to_string(),
            event_sender,
        );

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

        Self {
            state,
            store,
            base_url: format!("http://{addr}"),
        }
    }

    /// Starts a worker polling this API; it stops when the guard is dropped.
    fn start_worker(&self) -> WorkerGuard {
        let worker = WorkerBuilder::new(&self.base_url, WORKER_TOKEN)
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .register(Child)
            .register(GuardedRoot)
            .worker_id("worker-guard-on-worker")
            .concurrency(1)
            .poll_interval(Duration::from_millis(20))
            .lease_ttl(Duration::from_secs(5))
            .lease_refresh_interval(Duration::from_millis(500))
            .run_timeout(Duration::from_secs(15))
            .build()
            .expect("build worker");
        WorkerGuard(spawn(async move {
            if let Err(e) = worker.run().await {
                eprintln!("worker exited with error: {e:?}");
            }
        }))
    }

    async fn enqueue_root(&self) -> Uuid {
        self.state
            .engine
            .enqueue_handler(ROOT, TriggerKind::Manual, json!({}), 0)
            .await
            .expect("enqueue run")
            .id
    }

    /// Polls until the run is terminal, panicking at the deadline.
    async fn wait_until_terminal(&self, run_id: Uuid) -> Run {
        let deadline = Instant::now() + POLL_DEADLINE;
        loop {
            let run = self
                .store
                .get_run(run_id)
                .await
                .expect("get run")
                .expect("run exists");
            if run.status.state.is_terminal() {
                return run;
            }
            assert!(
                Instant::now() < deadline,
                "run {run_id} never finished, last status {:?}, error {:?}",
                run.status.state,
                run.error
            );
            sleep(Duration::from_millis(20)).await;
        }
    }

    /// Records an earlier child of `root_id` whose agent step spent `tokens`.
    ///
    /// The child requires a tag the worker does not carry, so the worker
    /// never picks it: it only stands for tokens already spent in the tree.
    async fn seed_child_tokens(&self, root_id: Uuid, tokens: u64) {
        let child = self
            .store
            .create_run(NewRun {
                created_by: None,
                workflow_name: CHILD.to_string(),
                trigger: TriggerKind::Workflow,
                payload: json!({}),
                max_retries: 0,
                handler_version: None,
                labels: HashMap::from([(LABEL_ROOT_RUN_ID.to_string(), root_id.to_string())]),
                scheduled_at: None,
                idempotency_key: None,
                concurrency_key: None,
                priority: 0,
                concurrency_limits: Vec::new(),
                max_cost_usd: None,
                worker_tags: vec!["elsewhere".to_string()],
            })
            .await
            .expect("create child")
            .into_run();
        let step = self
            .store
            .create_step(NewStep {
                run_id: child.id,
                trace_id: step_trace_id(child.id, "earlier-agent", 0),
                name: "earlier-agent".to_string(),
                kind: StepKind::Agent,
                position: 0,
                input: None,
                is_error_handler: false,
            })
            .await
            .expect("create step");
        self.store
            .update_step(
                step.id,
                StepUpdate {
                    input_tokens: Some(tokens),
                    ..StepUpdate::default()
                },
            )
            .await
            .expect("record tokens");
    }
}

/// Aborts the worker task when dropped.
struct WorkerGuard(JoinHandle<()>);

impl Drop for WorkerGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[tokio::test]
async fn guard_on_worker_runs_the_guarded_workflow_to_completion() {
    timeout(TEST_TIMEOUT, async {
        let api = Api::start().await;
        let _worker = api.start_worker();
        let root_id = api.enqueue_root().await;

        let root = api.wait_until_terminal(root_id).await;

        assert_eq!(
            root.status.state,
            RunStatus::Completed,
            "error: {:?}",
            root.error
        );
        let steps = api.store.list_steps(root_id).await.expect("list steps");
        let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["root-echo", CHILD]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn guard_on_worker_counts_tokens_spent_by_a_child() {
    timeout(TEST_TIMEOUT, async {
        let api = Api::start().await;
        // Seeded before the worker starts, so the root is claimed with the
        // child's tokens already in the tree.
        let root_id = api.enqueue_root().await;
        api.seed_child_tokens(root_id, TOKEN_BUDGET + 500).await;
        let _worker = api.start_worker();

        let root = api.wait_until_terminal(root_id).await;

        assert_eq!(root.status.state, RunStatus::Cancelled);
        let error = root.error.expect("a guard stop records its cause");
        assert!(error.contains("token budget exhausted"), "error: {error}");
        let steps = api.store.list_steps(root_id).await.expect("list steps");
        let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["root-echo"], "the child must not start");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn guard_on_worker_lists_runs_by_label_through_the_api() {
    timeout(TEST_TIMEOUT, async {
        let api = Api::start().await;
        let root_id = api.enqueue_root().await;
        api.seed_child_tokens(root_id, 1).await;
        let store = ApiRunStore::new(&api.base_url, WORKER_TOKEN);

        let filter = RunFilter {
            labels: Some(HashMap::from([(
                LABEL_ROOT_RUN_ID.to_string(),
                root_id.to_string(),
            )])),
            ..RunFilter::default()
        };
        let page = store.list_runs(filter, 1, 100).await.expect("list runs");

        assert_eq!(page.total, 1);
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].workflow_name, CHILD);

        let err = store
            .list_runs(RunFilter::default(), 0, 10)
            .await
            .expect_err("page 0 is invalid");
        assert!(err.to_string().contains("page must be at least 1"), "{err}");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn guard_on_worker_failure_before_the_first_step_records_the_real_error() {
    let run_id = Uuid::now_v7();
    // The stub has no run search: building the guard fails before any step.
    let state = Arc::new(TestApiState::new(vec![make_run_json(run_id, ROOT, 0)]));
    let api_url = spawn_test_api(state.clone()).await;

    let worker = WorkerBuilder::new(&api_url, "test-token")
        .provider(Arc::new(ClaudeCodeProvider::new()))
        .register(Child)
        .register(GuardedRoot)
        .worker_id("worker-test")
        .concurrency(1)
        .poll_interval(Duration::from_millis(20))
        .lease_ttl(Duration::from_millis(500))
        .lease_refresh_interval(Duration::from_millis(50))
        .run_timeout(Duration::from_secs(10))
        .build()
        .expect("build worker");

    if let Ok(Err(e)) = timeout(Duration::from_secs(3), worker.run()).await {
        eprintln!("worker exited with error: {e:?}");
    }

    assert!(
        state.created_steps.lock().unwrap().is_empty(),
        "no step may start"
    );
    let failure = state
        .all_status_bodies()
        .into_iter()
        .find(|b| b["status"] == "failed")
        .expect("the worker never recorded the failure on the run");
    let error = failure["error"]
        .as_str()
        .expect("the failure carries an error");
    assert!(error.contains("worker API error"), "error: {error}");
}
