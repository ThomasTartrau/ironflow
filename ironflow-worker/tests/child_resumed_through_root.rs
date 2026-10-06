//! Split API + worker deployment: a sub-workflow child suspended on a signal
//! is picked by a worker once the signal arrives, and the worker resumes its
//! root run through it.
//!
//! The worker holds the lease of the child it picked. The engine hands that
//! lease to the root it resumes, and the worker's refresher follows it: the
//! root keeps running past the end of the child instead of being abandoned
//! `Running` without a lease, invisible to the reaper.
//!
//! Everything is real: the router built by `create_router` served over TCP, an
//! `InMemoryStore`, an `Engine` in `ExecutionMode::Workers` on the API side and
//! a `Worker` polling that API.
//!
//! Test names contain `child_resumed_through_root` so `cargo test -p
//! ironflow-worker child_resumed_through_root` selects the whole suite.

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::serve;
use ironflow_api::routes::{RouterConfig, create_router};
use ironflow_api::state::AppState;
use ironflow_auth::jwt::JwtConfig;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, ExecutionMode};
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::notify::Event;
use ironflow_engine::signal::Signal;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{Run, RunFilter, RunStatus, TriggerKind};
use ironflow_store::store::RunStore;
use ironflow_worker::WorkerBuilder;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::net::TcpListener;
use tokio::spawn;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

/// Workflow name of [`Root`].
const ROOT: &str = "lease-root";

/// Workflow name of [`Waiter`].
const WAITER: &str = "lease-waiter";

/// Identifier of the worker, compared with the lease holder of the root.
const WORKER_ID: &str = "worker-child-resumed-through-root";

/// Key [`Waiter`] waits on.
const SIGNAL_KEY: &str = "release-183";

/// Lease refresh interval of the worker: short, so the refresher runs many
/// times while the root executes the step that follows the child.
const REFRESH_INTERVAL: Duration = Duration::from_millis(100);

/// Wall-clock budget for a whole test.
const TEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Deadline of every polling loop, generous for a slow CI executor.
const POLL_DEADLINE: Duration = Duration::from_secs(20);

/// Payload of a workflow that takes nothing.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct NoInput {}

/// The signal [`Waiter`] waits for.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Released {
    version: String,
}

impl Signal for Released {
    const NAME: &'static str = "test.released";
}

/// A child that waits for [`Released`] on [`SIGNAL_KEY`].
struct Waiter;

impl WorkflowHandler for Waiter {
    fn name(&self) -> &str {
        WAITER
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let released = ctx
                .wait_for_signal::<Released>("wait", SIGNAL_KEY, Duration::from_secs(3600))
                .await?;
            if released.is_none_or(|r| r.version.is_empty()) {
                return Err(EngineError::InvalidWorkflow(
                    "the release signal never arrived".to_string(),
                ));
            }
            Ok(())
        })
    }
}

impl TypedWorkflow for Waiter {
    type Input = NoInput;
}

/// Runs [`Waiter`], then a step that lasts many refresh intervals.
struct Root;

impl WorkflowHandler for Root {
    fn name(&self) -> &str {
        ROOT
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&Waiter, NoInput {}).await?;
            // 15 refresh intervals: before the fix the worker abandoned the
            // root at the first refresh after the child finished.
            ctx.shell("after-child", ShellConfig::new("sleep 1.5"))
                .await?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Root {
    type Input = NoInput;
}

/// A running API server and worker; the worker stops when this is dropped.
struct Harness {
    state: AppState,
    store: Arc<InMemoryStore>,
    worker: JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.worker.abort();
    }
}

impl Harness {
    async fn start() -> Self {
        let store = Arc::new(InMemoryStore::new());

        let mut engine = Engine::new(store.clone(), Arc::new(ClaudeCodeProvider::new()))
            .with_execution_mode(ExecutionMode::Workers);
        engine.register(Waiter).expect("register waiter");
        engine.register(Root).expect("register root");
        let jwt_config = Arc::new(JwtConfig {
            secret: "test-secret-for-child-resumed-through-root".to_string(),
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
            "test-worker-token".to_string(),
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
        let base_url = format!("http://{addr}");

        let worker = WorkerBuilder::new(&base_url, "test-worker-token")
            .provider(Arc::new(ClaudeCodeProvider::new()))
            .register(Waiter)
            .register(Root)
            .worker_id(WORKER_ID)
            .concurrency(1)
            .poll_interval(Duration::from_millis(20))
            .lease_ttl(Duration::from_secs(1))
            .lease_refresh_interval(REFRESH_INTERVAL)
            .run_timeout(Duration::from_secs(15))
            .build()
            .expect("build worker");
        let worker = spawn(async move {
            if let Err(e) = worker.run().await {
                eprintln!("worker exited with error: {e:?}");
            }
        });

        Self {
            state,
            store,
            worker,
        }
    }

    /// Enqueues a [`Root`] run; the worker claims it.
    async fn enqueue_root(&self) -> Uuid {
        self.state
            .engine
            .enqueue_handler(ROOT, TriggerKind::Manual, json!({}), 0)
            .await
            .expect("enqueue run")
            .id
    }

    async fn load(&self, run_id: Uuid) -> Run {
        self.store
            .get_run(run_id)
            .await
            .expect("get run")
            .expect("run exists")
    }

    /// Polls until the run reaches `expected`, panicking at the deadline.
    async fn wait_for_status(&self, run_id: Uuid, expected: RunStatus) -> Run {
        let deadline = Instant::now() + POLL_DEADLINE;
        let mut status = RunStatus::Pending;
        while Instant::now() < deadline {
            let run = self.load(run_id).await;
            status = run.status.state;
            if status == expected {
                return run;
            }
            sleep(Duration::from_millis(20)).await;
        }
        panic!("run {run_id} did not reach {expected:?} in time, last status {status:?}");
    }

    /// The single [`Waiter`] run.
    async fn waiter(&self) -> Run {
        let filter = RunFilter {
            workflow_name: Some(WAITER.to_string()),
            ..RunFilter::default()
        };
        let mut runs = self
            .store
            .list_runs(filter, 1, 50)
            .await
            .expect("list runs")
            .items;
        runs.retain(|r| r.workflow_name == WAITER);
        assert_eq!(runs.len(), 1, "expected exactly one waiter run");
        runs.remove(0)
    }

    /// Suspends the chain on the signal, then delivers it: the child is
    /// requeued and picked by the worker. Returns the root and child run ids.
    async fn suspend_then_release(&self) -> (Uuid, Uuid) {
        let root_id = self.enqueue_root().await;
        self.wait_for_status(root_id, RunStatus::Sleeping).await;
        let child = self.waiter().await;
        assert_eq!(child.status.state, RunStatus::Sleeping);

        let delivery = self
            .state
            .engine
            .send_signal(
                &Released {
                    version: "1.0.0".to_string(),
                },
                SIGNAL_KEY,
                None,
            )
            .await
            .expect("deliver");
        assert_eq!(delivery.resumed.len(), 1);
        assert_eq!(delivery.resumed[0].run_id, child.id);
        (root_id, child.id)
    }
}

#[tokio::test]
async fn child_resumed_through_root_completes_parent_without_losing_lease() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::start().await;
        let (root_id, child_id) = harness.suspend_then_release().await;

        // Watch the root until it finishes: once resumed it must always hold
        // the lease of this worker while it is `Running`.
        let deadline = Instant::now() + POLL_DEADLINE;
        let mut saw_leased_running = false;
        let root = loop {
            assert!(Instant::now() < deadline, "root run never finished");
            let root = harness.load(root_id).await;
            match root.status.state {
                RunStatus::Completed => break root,
                RunStatus::Running if root.worker_id.is_some() => {
                    assert_eq!(root.worker_id.as_deref(), Some(WORKER_ID));
                    assert!(root.lease_expires_at.is_some());
                    saw_leased_running = true;
                }
                RunStatus::Running => panic!("root run is Running without a lease"),
                state => assert!(
                    !state.is_terminal(),
                    "root run ended {state:?}, error {:?}",
                    root.error
                ),
            }
            sleep(Duration::from_millis(20)).await;
        };

        assert!(saw_leased_running, "the resumed root never held the lease");
        assert_eq!(root.lease_recoveries, 0, "the root was never reaped");
        assert!(root.worker_id.is_none(), "a finished run keeps no lease");
        let child = harness.load(child_id).await;
        assert_eq!(child.status.state, RunStatus::Completed);
        assert!(child.worker_id.is_none());

        let steps = harness.store.list_steps(root_id).await.expect("list steps");
        let names: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec![WAITER, "after-child"]);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn child_resumed_through_root_keeps_root_visible_to_reaper() {
    timeout(TEST_TIMEOUT, async {
        let harness = Harness::start().await;
        let (root_id, child_id) = harness.suspend_then_release().await;

        // The root runs its slow step once the child has finished.
        let deadline = Instant::now() + POLL_DEADLINE;
        let (root, child) = loop {
            assert!(Instant::now() < deadline, "root never ran past its child");
            let root = harness.load(root_id).await;
            let child = harness.load(child_id).await;
            if root.status.state == RunStatus::Running && child.status.state == RunStatus::Completed
            {
                break (root, child);
            }
            sleep(Duration::from_millis(20)).await;
        };

        assert_eq!(root.worker_id.as_deref(), Some(WORKER_ID));
        let first_expiry = root.lease_expires_at.expect("the root holds a lease");
        assert!(child.worker_id.is_none(), "the child gave its lease away");
        assert!(child.lease_expires_at.is_none());

        // The worker renews the root's lease, not the finished child's.
        sleep(REFRESH_INTERVAL * 4).await;
        let root = harness.load(root_id).await;
        assert_eq!(root.status.state, RunStatus::Running);
        assert_eq!(root.worker_id.as_deref(), Some(WORKER_ID));
        let renewed_expiry = root.lease_expires_at.expect("the root holds a lease");
        assert!(
            renewed_expiry > first_expiry,
            "the root lease is renewed by the worker"
        );

        // A valid lease is left alone by the reaper.
        let reaped = harness.store.reap_expired_leases(10).await.expect("reap");
        assert!(reaped.is_empty());

        harness.wait_for_status(root_id, RunStatus::Completed).await;
    })
    .await
    .expect("test timed out");
}
