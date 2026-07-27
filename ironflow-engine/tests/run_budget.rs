//! Integration tests for the per-run cost cap and the monthly cost quota.
//!
//! Agent steps replay recorded fixtures via [`RecordReplayProvider`], so every
//! run has a deterministic, known cost with no network or CLI involved.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use rust_decimal::Decimal;
use serde_json::json;

use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::{RecordReplayProvider, hash_config};
use ironflow_engine::budget::BudgetConfig;
use ironflow_engine::config::{AgentStepConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::{Engine, EnqueueOptions};
use ironflow_engine::error::{EngineError, RUN_BUDGET_EXCEEDED_CODE};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, RunUpdate, TriggerKind};
use ironflow_store::store::RunStore;

/// Declared budget of every agent step in these tests, in USD.
const STEP_BUDGET: f64 = 0.10;
/// Actual cost the replayed fixture reports for each agent step, in USD.
const STEP_COST: f64 = 0.10;

/// Removes the fixtures directory when the test ends.
struct FixtureGuard {
    dir: String,
}

impl Drop for FixtureGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// The agent config used by every step, so a single fixture serves them all.
fn agent_config() -> AgentConfig {
    AgentStepConfig::new("summarize the build")
        .model("haiku")
        .max_budget_usd(STEP_BUDGET)
}

/// Write the replay fixture and return its directory plus a cleanup guard.
fn fixtures_dir(name: &str) -> (String, FixtureGuard) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = format!(
        "/tmp/ironflow-budget-fixtures-{}-{}-{}",
        name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
    );
    let guard = FixtureGuard { dir: dir.clone() };

    let config = agent_config();
    let mut output = AgentOutput::new(json!("done"));
    output.cost_usd = Some(STEP_COST);
    output.duration_ms = 10;

    fs::create_dir_all(&dir).expect("create fixtures dir");
    let path = PathBuf::from(&dir).join(format!("{}.json", hash_config(&config)));
    let fixture = json!({ "config": config, "output": output });
    fs::write(
        path,
        serde_json::to_string(&fixture).expect("serialize fixture"),
    )
    .expect("write fixture");

    (dir, guard)
}

fn engine_with(store: Arc<InMemoryStore>, dir: &str) -> Engine {
    let provider: Arc<dyn AgentProvider> =
        Arc::new(RecordReplayProvider::replay(ClaudeCodeProvider::new(), dir));
    Engine::new(store, provider)
}

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

/// Runs `n` sequential agent steps, each declaring `STEP_BUDGET`.
struct SequentialAgents {
    n: usize,
}

impl WorkflowHandler for SequentialAgents {
    fn name(&self) -> &str {
        "sequential-agents"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            for i in 0..self.n {
                ctx.agent(&format!("step-{i}"), agent_config()).await?;
            }
            Ok(())
        })
    }
}

/// Runs one wave of `n` concurrent agent steps.
struct ParallelAgents {
    n: usize,
}

impl WorkflowHandler for ParallelAgents {
    fn name(&self) -> &str {
        "parallel-agents"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let steps: Vec<(&str, StepConfig)> = (0..self.n)
                .map(|_| ("agent", StepConfig::Agent(agent_config())))
                .collect();
            ctx.parallel(steps, true).await?;
            Ok(())
        })
    }
}

/// Sub-workflow with a single agent step.
struct ChildOneAgent;

impl WorkflowHandler for ChildOneAgent {
    fn name(&self) -> &str {
        "child-one-agent"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("child-step", agent_config()).await?;
            Ok(())
        })
    }
}

/// Calls the child sub-workflow, then runs `n` agent steps of its own.
struct ParentWithChild {
    n: usize,
}

impl WorkflowHandler for ParentWithChild {
    fn name(&self) -> &str {
        "parent-with-child"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&ChildOneAgent, json!({})).await?;
            for i in 0..self.n {
                ctx.agent(&format!("parent-step-{i}"), agent_config())
                    .await?;
            }
            Ok(())
        })
    }
}

/// Only shell steps -- never charged against the cap.
struct ShellOnly;

impl WorkflowHandler for ShellOnly {
    fn name(&self) -> &str {
        "shell-only"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("echo", ShellConfig::new("echo hi")).await?;
            ctx.shell("echo-again", ShellConfig::new("echo hi again"))
                .await?;
            Ok(())
        })
    }
}

// ---------------------------------------------------------------------------
// Per-run cost cap
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_stops_exactly_when_the_next_step_would_cross_the_cap() {
    let (dir, _guard) = fixtures_dir("exact-stop");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(SequentialAgents { n: 3 }).unwrap();

    // Cap of $0.20 with $0.10 steps: two steps fit, the third is refused.
    let run = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::new(20, 2)),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();

    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("third step must be refused");
    assert!(matches!(err, EngineError::RunBudgetExceeded { .. }));
    assert!(err.to_string().contains(RUN_BUDGET_EXCEEDED_CODE));

    // Only the two affordable steps were ever created.
    let steps = store.list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 2, "the refused step must never be recorded");

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Cancelled);
    assert_eq!(final_run.cost_usd, Decimal::new(20, 2));
    assert!(
        final_run
            .error
            .as_deref()
            .unwrap_or_default()
            .contains(RUN_BUDGET_EXCEEDED_CODE)
    );
}

#[tokio::test]
async fn run_without_cap_executes_every_step() {
    let (dir, _guard) = fixtures_dir("no-cap");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(SequentialAgents { n: 3 }).unwrap();

    let run = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions::default(),
        )
        .await
        .expect("enqueue")
        .into_run();
    assert!(run.max_cost_usd.is_none());

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    engine.execute_handler_run(run.id).await.expect("run");

    let steps = store.list_steps(run.id).await.unwrap();
    assert_eq!(steps.len(), 3);

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Completed);
    assert_eq!(final_run.cost_usd, Decimal::new(30, 2));
}

#[tokio::test]
async fn run_completes_when_the_cap_is_exactly_met() {
    let (dir, _guard) = fixtures_dir("exact-fit");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(SequentialAgents { n: 2 }).unwrap();

    let run = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::new(20, 2)),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    engine.execute_handler_run(run.id).await.expect("run");

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Completed);
    assert_eq!(final_run.cost_usd, Decimal::new(20, 2));
}

#[tokio::test]
async fn zero_cap_refuses_the_first_agent_step() {
    let (dir, _guard) = fixtures_dir("zero-cap");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(SequentialAgents { n: 1 }).unwrap();

    let run = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::ZERO),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("first step must be refused");
    assert!(matches!(err, EngineError::RunBudgetExceeded { .. }));

    assert!(store.list_steps(run.id).await.unwrap().is_empty());
    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Cancelled);
}

#[tokio::test]
async fn zero_cap_does_not_block_non_agent_steps() {
    let (dir, _guard) = fixtures_dir("shell-under-cap");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(ShellOnly).unwrap();

    let run = engine
        .enqueue_handler_with_options(
            "shell-only",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::ZERO),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    engine.execute_handler_run(run.id).await.expect("run");

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Completed);
    assert_eq!(store.list_steps(run.id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn parallel_wave_is_refused_as_a_whole_and_creates_no_step() {
    let (dir, _guard) = fixtures_dir("parallel-refused");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(ParallelAgents { n: 3 }).unwrap();

    // Wave budget is 3 x $0.10 = $0.30, above the $0.20 cap.
    let run = engine
        .enqueue_handler_with_options(
            "parallel-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::new(20, 2)),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("wave must be refused");
    assert!(matches!(err, EngineError::RunBudgetExceeded { .. }));

    assert!(
        store.list_steps(run.id).await.unwrap().is_empty(),
        "no step of a refused wave may be recorded"
    );
    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Cancelled);
}

#[tokio::test]
async fn parallel_wave_runs_when_the_summed_budget_fits() {
    let (dir, _guard) = fixtures_dir("parallel-allowed");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(ParallelAgents { n: 2 }).unwrap();

    let run = engine
        .enqueue_handler_with_options(
            "parallel-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::new(20, 2)),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    engine.execute_handler_run(run.id).await.expect("run");

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Completed);
    assert_eq!(store.list_steps(run.id).await.unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// Sub-workflow propagation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn sub_workflow_cost_counts_against_the_parent_cap() {
    let (dir, _guard) = fixtures_dir("child-counts");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(ParentWithChild { n: 2 }).unwrap();
    engine.register(ChildOneAgent).unwrap();

    // Child spends $0.10, then the parent's first step spends $0.10 (cap met).
    // The parent's second step must be refused.
    let run = engine
        .enqueue_handler_with_options(
            "parent-with-child",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::new(20, 2)),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("parent must run out of budget");
    assert!(matches!(err, EngineError::RunBudgetExceeded { .. }));

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Cancelled);
    assert_eq!(final_run.cost_usd, Decimal::new(20, 2));
}

#[tokio::test]
async fn sub_workflow_inherits_the_parent_cap_and_stops_inside_the_child() {
    let (dir, _guard) = fixtures_dir("child-refused");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(ParentWithChild { n: 0 }).unwrap();
    engine.register(ChildOneAgent).unwrap();

    // A zero cap leaves nothing for the child's own agent step.
    let run = engine
        .enqueue_handler_with_options(
            "parent-with-child",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::ZERO),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("child must be refused");
    assert!(matches!(err, EngineError::RunBudgetExceeded { .. }));

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Cancelled);

    // The child run carries the same cap and failed on its own step.
    let children = store
        .list_runs(Default::default(), 1, 100)
        .await
        .unwrap()
        .items
        .into_iter()
        .filter(|r| r.workflow_name == "child-one-agent")
        .collect::<Vec<_>>();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].max_cost_usd, Some(Decimal::ZERO));
}

#[tokio::test]
async fn sub_workflow_completes_when_the_shared_cap_has_room() {
    let (dir, _guard) = fixtures_dir("child-allowed");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(ParentWithChild { n: 1 }).unwrap();
    engine.register(ChildOneAgent).unwrap();

    let run = engine
        .enqueue_handler_with_options(
            "parent-with-child",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_cost_usd: Some(Decimal::new(20, 2)),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    engine.execute_handler_run(run.id).await.expect("run");

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(final_run.status.state, RunStatus::Completed);
    assert_eq!(final_run.cost_usd, Decimal::new(20, 2));
}

// ---------------------------------------------------------------------------
// Cap resolution
// ---------------------------------------------------------------------------

/// Declares its own default cap, overriding the server default.
struct CappedHandler;

impl WorkflowHandler for CappedHandler {
    fn name(&self) -> &str {
        "capped-handler"
    }

    fn default_max_cost_usd(&self) -> Option<Decimal> {
        Some(Decimal::new(200, 2))
    }

    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

#[tokio::test]
async fn cap_resolution_prefers_request_then_handler_then_server() {
    let (dir, _guard) = fixtures_dir("cap-resolution");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir)
        .with_budget_config(BudgetConfig::new().default_run_max_cost_usd(Decimal::new(100, 2)));
    engine.register(CappedHandler).unwrap();
    engine.register(SequentialAgents { n: 0 }).unwrap();

    let enqueue = async |workflow: &str, requested: Option<Decimal>| {
        engine
            .enqueue_handler_with_options(
                workflow,
                TriggerKind::Api,
                json!({}),
                EnqueueOptions {
                    max_cost_usd: requested,
                    ..Default::default()
                },
            )
            .await
            .expect("enqueue")
            .into_run()
            .max_cost_usd
    };

    // Request wins over both defaults.
    assert_eq!(
        enqueue("capped-handler", Some(Decimal::new(300, 2))).await,
        Some(Decimal::new(300, 2))
    );
    // Handler default wins over the server default.
    assert_eq!(
        enqueue("capped-handler", None).await,
        Some(Decimal::new(200, 2))
    );
    // Server default applies when nothing else declares a cap.
    assert_eq!(
        enqueue("sequential-agents", None).await,
        Some(Decimal::new(100, 2))
    );
}

// ---------------------------------------------------------------------------
// Monthly quota
// ---------------------------------------------------------------------------

#[tokio::test]
async fn monthly_quota_blocks_new_runs_without_touching_running_ones() {
    let (dir, _guard) = fixtures_dir("monthly-quota");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir)
        .with_budget_config(BudgetConfig::new().monthly_cost_limit_usd(Decimal::new(50, 2)));
    engine.register(SequentialAgents { n: 1 }).unwrap();

    // A first run is allowed and burns the whole quota.
    let first = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions::default(),
        )
        .await
        .expect("first run allowed")
        .into_run();

    store
        .update_run_status(first.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run(
            first.id,
            RunUpdate {
                cost_usd: Some(Decimal::new(60, 2)),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    // The quota is now exhausted: creation is refused.
    let err = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions::default(),
        )
        .await
        .expect_err("second run must be refused");
    assert!(matches!(err, EngineError::MonthlyBudgetExceeded { .. }));

    // The in-flight run is untouched.
    let still_running = store.get_run(first.id).await.unwrap().expect("run exists");
    assert_eq!(still_running.status.state, RunStatus::Running);
}

#[tokio::test]
async fn monthly_quota_allows_runs_while_there_is_room() {
    let (dir, _guard) = fixtures_dir("monthly-room");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir)
        .with_budget_config(BudgetConfig::new().monthly_cost_limit_usd(Decimal::new(10000, 2)));
    engine.register(SequentialAgents { n: 1 }).unwrap();

    for _ in 0..3 {
        engine
            .enqueue_handler_with_options(
                "sequential-agents",
                TriggerKind::Api,
                json!({}),
                EnqueueOptions::default(),
            )
            .await
            .expect("run allowed while quota has room")
            .into_run();
    }
}

#[tokio::test]
async fn unconfigured_monthly_quota_never_blocks() {
    let (dir, _guard) = fixtures_dir("monthly-unset");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(SequentialAgents { n: 1 }).unwrap();

    let run = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions::default(),
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    store
        .update_run(
            run.id,
            RunUpdate {
                cost_usd: Some(Decimal::new(999999, 2)),
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions::default(),
        )
        .await
        .expect("no quota configured, creation must succeed")
        .into_run();
}

// ---------------------------------------------------------------------------
// Cost cap vs automatic retry
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cap_refusal_cancels_the_run_even_with_retries_left() {
    let (dir, _guard) = fixtures_dir("cap-vs-retry");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(SequentialAgents { n: 3 }).unwrap();

    // Cap of $0.20 with $0.10 steps: the third step is refused while the run
    // still has two retries in the bank.
    let run = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_retries: 2,
                max_cost_usd: Some(Decimal::new(20, 2)),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();

    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("third step must be refused");
    assert!(matches!(err, EngineError::RunBudgetExceeded { .. }));

    let final_run = store.get_run(run.id).await.unwrap().expect("run exists");
    assert_eq!(
        final_run.status.state,
        RunStatus::Cancelled,
        "a cap refusal must never arm a retry"
    );
    assert_eq!(final_run.retry_count, 0, "no attempt may be consumed");
    assert!(
        final_run.scheduled_at.is_none(),
        "no backoff may be scheduled"
    );
}

#[tokio::test]
async fn the_cap_covers_every_attempt_of_a_run() {
    let (dir, _guard) = fixtures_dir("cap-across-attempts");
    let store = Arc::new(InMemoryStore::new());
    let mut engine = engine_with(store.clone(), &dir);
    engine.register(SequentialAgents { n: 3 }).unwrap();

    let run = engine
        .enqueue_handler_with_options(
            "sequential-agents",
            TriggerKind::Api,
            json!({}),
            EnqueueOptions {
                max_retries: 1,
                max_cost_usd: Some(Decimal::new(20, 2)),
                ..Default::default()
            },
        )
        .await
        .expect("enqueue")
        .into_run();

    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .unwrap();
    // As if a first attempt had already spent $0.15 of the $0.20 cap.
    store
        .update_run(
            run.id,
            RunUpdate {
                cost_usd: Some(Decimal::new(15, 2)),
                increment_retry: true,
                ..RunUpdate::default()
            },
        )
        .await
        .unwrap();

    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("the replay must not get a fresh budget");
    assert!(matches!(err, EngineError::RunBudgetExceeded { .. }));

    let steps = store.list_steps(run.id).await.unwrap();
    assert!(
        steps.is_empty(),
        "no step of the new attempt may run on an exhausted cap"
    );
}
