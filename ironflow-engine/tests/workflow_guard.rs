//! Integration tests for the [`WorkflowGuard`](ironflow_engine::guard).
//!
//! Tests exercise the guard through the [`Engine`] to verify that limits
//! are enforced end-to-end when running real (non-mocked) sub-workflows.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use chrono::{TimeDelta, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::time::{sleep, timeout};
use uuid::Uuid;

use ironflow_core::error::OperationError;
use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::{RecordReplayProvider, hash_config};
use ironflow_engine::config::{
    AgentStepConfig, ApprovalConfig, HumanInputConfig, ShellConfig, StepConfig,
};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::guard::{WORKFLOW_GUARD_REJECTED_CODE, WorkflowGuardConfig};
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use ironflow_engine::signal::Signal;
use ironflow_engine::testing::MissingAgentProvider;
use ironflow_engine::wake::RunWaker;
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{
    Run, RunFilter, RunStatus, RunUpdate, Step, StepApproval, StepKind, StepStatus, StepUpdate,
    TriggerKind,
};
use ironflow_store::store::RunStore;

// ---------------------------------------------------------------------------
// Test handlers
// ---------------------------------------------------------------------------

/// A workflow that calls a sub-workflow, which itself calls another.
/// Used to test depth limits: root -> child -> grandchild.
struct RootWorkflow;

impl WorkflowHandler for RootWorkflow {
    fn name(&self) -> &str {
        "root"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&ChildWorkflow, ()).await?;
            Ok(())
        })
    }
}

struct ChildWorkflow;

impl TypedWorkflow for ChildWorkflow {
    type Input = ();
}

impl WorkflowHandler for ChildWorkflow {
    fn name(&self) -> &str {
        "child"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&GrandchildWorkflow, ()).await?;
            Ok(())
        })
    }
}

struct GrandchildWorkflow;

impl TypedWorkflow for GrandchildWorkflow {
    type Input = ();
}

impl WorkflowHandler for GrandchildWorkflow {
    fn name(&self) -> &str {
        "grandchild"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("echo", ShellConfig::new("echo done")).await?;
            Ok(())
        })
    }
}

/// A workflow that creates a cycle: A -> B -> A.
struct CycleA;

impl TypedWorkflow for CycleA {
    type Input = ();
}

impl WorkflowHandler for CycleA {
    fn name(&self) -> &str {
        "cycle-a"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&CycleB, ()).await?;
            Ok(())
        })
    }
}

struct CycleB;

impl TypedWorkflow for CycleB {
    type Input = ();
}

impl WorkflowHandler for CycleB {
    fn name(&self) -> &str {
        "cycle-b"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&CycleA, ()).await?;
            Ok(())
        })
    }
}

/// A workflow that spawns many sub-workflows in sequence to test fan-out.
struct FanOutWorkflow {
    count: usize,
}

impl WorkflowHandler for FanOutWorkflow {
    fn name(&self) -> &str {
        "fan-out"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            for _ in 0..self.count {
                ctx.workflow(&GrandchildWorkflow, ()).await?;
            }
            Ok(())
        })
    }
}

/// A workflow that declares its own strict guard config.
struct StrictGuardWorkflow;

impl WorkflowHandler for StrictGuardWorkflow {
    fn name(&self) -> &str {
        "strict-guard"
    }

    fn guard_config(&self) -> Option<WorkflowGuardConfig> {
        Some(WorkflowGuardConfig::new().with_max_depth(1))
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&ChildWorkflow, ()).await?;
            Ok(())
        })
    }
}

/// A workflow that has a permissive guard config.
struct PermissiveGuardWorkflow;

impl WorkflowHandler for PermissiveGuardWorkflow {
    fn name(&self) -> &str {
        "permissive-guard"
    }

    fn guard_config(&self) -> Option<WorkflowGuardConfig> {
        Some(WorkflowGuardConfig::new().with_max_depth(10))
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&ChildWorkflow, ()).await?;
            Ok(())
        })
    }
}

fn engine_with_guard(store: Arc<InMemoryStore>, config: WorkflowGuardConfig) -> Engine {
    let provider = Arc::new(ClaudeCodeProvider::new());
    Engine::new(store, provider).with_guard_config(config)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn depth_exceeded_stops_sub_workflow() {
    let store = Arc::new(InMemoryStore::new());
    let guard = WorkflowGuardConfig::new().with_max_depth(1);
    let mut engine = engine_with_guard(store.clone(), guard);

    engine.register(RootWorkflow).unwrap();
    engine.register(ChildWorkflow).unwrap();
    engine.register(GrandchildWorkflow).unwrap();

    let err = engine
        .run_handler("root", TriggerKind::Manual, json!({}))
        .await
        .expect_err("should be rejected by guard");

    assert!(
        matches!(err, EngineError::WorkflowGuardRejected(_)),
        "expected WorkflowGuardRejected, got {err:?}"
    );
    assert!(err.to_string().contains(WORKFLOW_GUARD_REJECTED_CODE));
    assert!(err.to_string().contains("max call depth exceeded"));
}

#[tokio::test]
async fn depth_within_limit_succeeds() {
    let store = Arc::new(InMemoryStore::new());
    let guard = WorkflowGuardConfig::new().with_max_depth(5);
    let mut engine = engine_with_guard(store.clone(), guard);

    engine.register(RootWorkflow).unwrap();
    engine.register(ChildWorkflow).unwrap();
    engine.register(GrandchildWorkflow).unwrap();

    let run = engine
        .run_handler("root", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(
        run.run.status.state,
        RunStatus::Completed,
        "run should succeed with max_depth=5 (chain is only 2 deep)"
    );
}

#[tokio::test]
async fn cycle_detected_rejects_invocation() {
    let store = Arc::new(InMemoryStore::new());
    let guard = WorkflowGuardConfig::new().with_max_depth(10);
    let mut engine = engine_with_guard(store.clone(), guard);

    engine.register(CycleA).unwrap();
    engine.register(CycleB).unwrap();

    let err = engine
        .run_handler("cycle-a", TriggerKind::Manual, json!({}))
        .await
        .expect_err("should be rejected by cycle detection");

    assert!(
        matches!(err, EngineError::WorkflowGuardRejected(_)),
        "expected WorkflowGuardRejected, got {err:?}"
    );
    assert!(err.to_string().contains("cycle detected"));
}

#[tokio::test]
async fn fan_out_exceeded_stops_workflow() {
    let store = Arc::new(InMemoryStore::new());
    let guard = WorkflowGuardConfig::new()
        .with_max_depth(5)
        .with_max_fan_out(3);
    let mut engine = engine_with_guard(store.clone(), guard);

    engine.register(FanOutWorkflow { count: 5 }).unwrap();
    engine.register(GrandchildWorkflow).unwrap();

    let err = engine
        .run_handler("fan-out", TriggerKind::Manual, json!({}))
        .await
        .expect_err("should be rejected by fan-out limit");

    assert!(
        matches!(err, EngineError::WorkflowGuardRejected(_)),
        "expected WorkflowGuardRejected, got {err:?}"
    );
    assert!(err.to_string().contains("max fan-out exceeded"));
}

#[tokio::test]
async fn fan_out_within_limit_succeeds() {
    let store = Arc::new(InMemoryStore::new());
    let guard = WorkflowGuardConfig::new()
        .with_max_depth(5)
        .with_max_fan_out(10);
    let mut engine = engine_with_guard(store.clone(), guard);

    engine.register(FanOutWorkflow { count: 3 }).unwrap();
    engine.register(GrandchildWorkflow).unwrap();

    let run = engine
        .run_handler("fan-out", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(
        run.run.status.state,
        RunStatus::Completed,
        "run should succeed with max_fan_out=10 (only 3 invocations)"
    );
}

#[tokio::test]
async fn handler_guard_config_overrides_global() {
    let store = Arc::new(InMemoryStore::new());
    // Global config allows deep nesting.
    let global_guard = WorkflowGuardConfig::new().with_max_depth(10);
    let mut engine = engine_with_guard(store.clone(), global_guard);

    engine.register(StrictGuardWorkflow).unwrap();
    engine.register(ChildWorkflow).unwrap();
    engine.register(GrandchildWorkflow).unwrap();

    // StrictGuardWorkflow has max_depth=1, so child -> grandchild is rejected.
    let err = engine
        .run_handler("strict-guard", TriggerKind::Manual, json!({}))
        .await
        .expect_err("handler guard_config should override global");

    assert!(
        matches!(err, EngineError::WorkflowGuardRejected(_)),
        "expected WorkflowGuardRejected, got {err:?}"
    );
    assert!(err.to_string().contains("max call depth exceeded"));
}

#[tokio::test]
async fn handler_guard_config_more_permissive_than_global() {
    let store = Arc::new(InMemoryStore::new());
    // Global config restricts depth.
    let global_guard = WorkflowGuardConfig::new().with_max_depth(1);
    let mut engine = engine_with_guard(store.clone(), global_guard);

    engine.register(PermissiveGuardWorkflow).unwrap();
    engine.register(ChildWorkflow).unwrap();
    engine.register(GrandchildWorkflow).unwrap();

    // PermissiveGuardWorkflow has max_depth=10, overriding global max_depth=1.
    let run = engine
        .run_handler("permissive-guard", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(
        run.run.status.state,
        RunStatus::Completed,
        "handler guard_config (max_depth=10) should override global (max_depth=1)"
    );
}

#[tokio::test]
async fn no_guard_config_allows_everything() {
    let store = Arc::new(InMemoryStore::new());
    let provider = Arc::new(ClaudeCodeProvider::new());
    let mut engine = Engine::new(store.clone(), provider);

    engine.register(RootWorkflow).unwrap();
    engine.register(ChildWorkflow).unwrap();
    engine.register(GrandchildWorkflow).unwrap();

    let run = engine
        .run_handler("root", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(
        run.run.status.state,
        RunStatus::Completed,
        "without guard config, no limits are enforced"
    );
}

// ---------------------------------------------------------------------------
// Token budget tests
// ---------------------------------------------------------------------------

const STEP_BUDGET: f64 = 0.10;

struct FixtureGuard {
    dir: String,
}

impl Drop for FixtureGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn agent_config() -> AgentConfig {
    AgentStepConfig::new("summarize the build")
        .model("haiku")
        .max_budget_usd(STEP_BUDGET)
}

fn fixtures_dir_with_tokens(
    name: &str,
    input_tokens: u64,
    output_tokens: u64,
) -> (String, FixtureGuard) {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = format!(
        "/tmp/ironflow-guard-fixtures-{}-{}-{}",
        name,
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed),
    );
    let guard = FixtureGuard { dir: dir.clone() };

    let config = agent_config();
    let mut output = AgentOutput::new(json!("done"));
    output.cost_usd = Some(0.10);
    output.duration_ms = 10;
    output.input_tokens = Some(input_tokens);
    output.output_tokens = Some(output_tokens);

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

/// Runs `n` sequential agent steps.
struct TokenAgents {
    n: usize,
}

impl WorkflowHandler for TokenAgents {
    fn name(&self) -> &str {
        "token-agents"
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

/// Runs `n` concurrent agent steps in a single parallel wave.
struct ParallelTokenAgents {
    n: usize,
}

impl WorkflowHandler for ParallelTokenAgents {
    fn name(&self) -> &str {
        "parallel-token-agents"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let names: Vec<String> = (0..self.n).map(|i| format!("agent-{i}")).collect();
            let steps: Vec<(&str, StepConfig)> = names
                .iter()
                .map(|name| (name.as_str(), StepConfig::Agent(agent_config())))
                .collect();
            ctx.parallel(steps, true).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn token_budget_exceeded_stops_sequential_agents() {
    // Each step uses 500+200=700 tokens. Budget is 1500, so 2 steps fit, 3rd is rejected.
    let (dir, _guard) = fixtures_dir_with_tokens("token-seq", 500, 200);
    let store = Arc::new(InMemoryStore::new());
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &dir,
    ));
    let guard = WorkflowGuardConfig::new().with_max_workflow_tokens(1500);
    let mut engine = Engine::new(store, provider).with_guard_config(guard);
    engine.register(TokenAgents { n: 3 }).unwrap();

    let err = engine
        .run_handler("token-agents", TriggerKind::Manual, json!({}))
        .await
        .expect_err("should be rejected by token budget");

    assert!(
        matches!(err, EngineError::WorkflowGuardRejected(_)),
        "expected WorkflowGuardRejected, got {err:?}"
    );
    assert!(err.to_string().contains("token budget exhausted"));
}

#[tokio::test]
async fn token_budget_within_limit_succeeds() {
    let (dir, _guard) = fixtures_dir_with_tokens("token-ok", 500, 200);
    let store = Arc::new(InMemoryStore::new());
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &dir,
    ));
    let guard = WorkflowGuardConfig::new().with_max_workflow_tokens(5000);
    let mut engine = Engine::new(store, provider).with_guard_config(guard);
    engine.register(TokenAgents { n: 3 }).unwrap();

    let run = engine
        .run_handler("token-agents", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.run.status.state, RunStatus::Completed);
}

#[tokio::test]
async fn token_budget_exceeded_in_parallel_wave() {
    // Each step uses 500+200=700 tokens. Budget is 1500. 3 parallel steps = 2100 > 1500.
    let (dir, _guard) = fixtures_dir_with_tokens("token-par", 500, 200);
    let store = Arc::new(InMemoryStore::new());
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::replay(
        ClaudeCodeProvider::new(),
        &dir,
    ));
    let guard = WorkflowGuardConfig::new().with_max_workflow_tokens(1500);
    let mut engine = Engine::new(store, provider).with_guard_config(guard);
    engine.register(ParallelTokenAgents { n: 3 }).unwrap();

    let err = engine
        .run_handler("parallel-token-agents", TriggerKind::Manual, json!({}))
        .await
        .expect_err("should be rejected by token budget in parallel");

    assert!(
        matches!(err, EngineError::WorkflowGuardRejected(_)),
        "expected WorkflowGuardRejected, got {err:?}"
    );
    assert!(err.to_string().contains("token budget exhausted"));
}

// ---------------------------------------------------------------------------
// Timeout tests
// ---------------------------------------------------------------------------

/// A workflow that sleeps longer than its timeout.
struct SlowWorkflow;

impl WorkflowHandler for SlowWorkflow {
    fn name(&self) -> &str {
        "slow-workflow"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("slow", ShellConfig::new("sleep 10")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn workflow_timeout_interrupts_slow_step() {
    let store = Arc::new(InMemoryStore::new());
    let guard = WorkflowGuardConfig::new().with_workflow_timeout_secs(1);
    let mut engine = engine_with_guard(store.clone(), guard);
    engine.register(SlowWorkflow).unwrap();

    let start = Instant::now();
    let err = engine
        .run_handler("slow-workflow", TriggerKind::Manual, json!({}))
        .await
        .expect_err("should be interrupted by timeout");

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 5,
        "timeout should have interrupted the sleep, but elapsed {elapsed:?}"
    );
    assert!(
        matches!(err, EngineError::WorkflowGuardRejected(_)),
        "expected WorkflowGuardRejected, got {err:?}"
    );
    assert!(err.to_string().contains("workflow timeout"));
}

#[tokio::test]
async fn workflow_timeout_allows_fast_step() {
    let store = Arc::new(InMemoryStore::new());
    let guard = WorkflowGuardConfig::new().with_workflow_timeout_secs(30);
    let mut engine = engine_with_guard(store.clone(), guard);
    engine.register(GrandchildWorkflow).unwrap();

    let run = engine
        .run_handler("grandchild", TriggerKind::Manual, json!({}))
        .await
        .unwrap();

    assert_eq!(run.run.status.state, RunStatus::Completed);
}

// ---------------------------------------------------------------------------
// Limits across executions of a run (#241)
// ---------------------------------------------------------------------------

/// Test timeout for bodies that wait for a resumed or stopped run.
const TEST_TIMEOUT: Duration = Duration::from_secs(10);

/// The signal [`SignalWaiter`] and [`SignalTokenAgents`] wait for.
#[derive(Debug, Serialize, Deserialize, JsonSchema)]
struct Release {
    version: String,
}

impl Signal for Release {
    const NAME: &'static str = "guard.release";
}

/// The answer of a human input step.
#[derive(Debug, Deserialize, JsonSchema)]
struct Named {
    name: String,
}

/// An engine replaying the fixtures of `dir` with a 1500-token budget.
fn token_engine(store: &Arc<InMemoryStore>, dir: &str) -> Engine {
    let guard = WorkflowGuardConfig::new().with_max_workflow_tokens(1500);
    let engine = Engine::new(store.clone(), replay_provider(dir));
    engine.with_guard_config(guard)
}

/// A replay provider that fails loudly on a missing fixture.
fn replay_provider(dir: &str) -> Arc<dyn AgentProvider> {
    Arc::new(RecordReplayProvider::replay(MissingAgentProvider, dir))
}

/// The stored run `run_id`.
async fn stored_run(store: &InMemoryStore, run_id: Uuid) -> Run {
    store
        .get_run(run_id)
        .await
        .expect("get run")
        .expect("run exists")
}

/// The steps of `run_id` of the given kind.
async fn steps_of_kind(store: &InMemoryStore, run_id: Uuid, kind: StepKind) -> Vec<Step> {
    store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .filter(|s| s.kind == kind)
        .collect()
}

/// The single run of `workflow`.
async fn run_of(store: &InMemoryStore, workflow: &str) -> Run {
    let mut runs = store
        .list_runs(
            RunFilter {
                workflow_name: Some(workflow.to_string()),
                ..RunFilter::default()
            },
            1,
            10,
        )
        .await
        .expect("list runs")
        .items;
    assert_eq!(runs.len(), 1, "expected exactly one {workflow} run");
    runs.remove(0)
}

/// Poll the store until the run reaches `status`.
async fn wait_for_status(store: &InMemoryStore, run_id: Uuid, status: RunStatus) {
    timeout(TEST_TIMEOUT, async {
        loop {
            if stored_run(store, run_id).await.status.state == status {
                return;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("run never reached the expected status");
}

/// Approve the open gate of `run_id` and mark the run running, like the
/// approval API does.
async fn approve_gate(store: &InMemoryStore, run_id: Uuid) {
    let gate = store
        .list_steps(run_id)
        .await
        .expect("list steps")
        .into_iter()
        .find(|s| s.status.state == StepStatus::AwaitingApproval)
        .expect("the gate is open");
    store
        .record_step_approval(
            gate.id,
            StepApproval {
                user_id: Uuid::now_v7(),
                approved_by: "tester".to_string(),
                at: Utc::now(),
            },
        )
        .await
        .expect("vote");
    store
        .update_run_status(run_id, RunStatus::Running)
        .await
        .expect("to running");
}

/// Answer the open human input of `run_id` and mark the run running, like
/// the API does.
async fn answer_input(store: &InMemoryStore, run_id: Uuid, name: &str) {
    let mut inputs = steps_of_kind(store, run_id, StepKind::HumanInput).await;
    assert_eq!(inputs.len(), 1, "expected exactly one human input step");
    let step = inputs.remove(0);
    assert_eq!(step.status.state, StepStatus::AwaitingApproval);
    store
        .update_step(
            step.id,
            StepUpdate {
                status: Some(StepStatus::Completed),
                output: Some(json!({ "name": name })),
                completed_at: Some(Utc::now()),
                clear_approval_deadline: true,
                ..StepUpdate::default()
            },
        )
        .await
        .expect("store the answer");
    store
        .update_run_status(run_id, RunStatus::Running)
        .await
        .expect("mark running");
}

/// Rewind the retry backoff of `run_id` and claim it, like a worker does.
async fn fast_forward_backoff(store: &InMemoryStore, run_id: Uuid) {
    store
        .update_run(
            run_id,
            RunUpdate {
                scheduled_at: Some(Utc::now() - TimeDelta::seconds(1)),
                ..RunUpdate::default()
            },
        )
        .await
        .expect("rewind scheduled_at");
    let picked = store
        .pick_next_pending(None)
        .await
        .expect("pick")
        .expect("a run waiting for its retry");
    assert_eq!(picked.id, run_id);
}

fn assert_token_rejection(err: &EngineError) {
    assert!(
        matches!(err, EngineError::WorkflowGuardRejected(_)),
        "expected WorkflowGuardRejected, got {err:?}"
    );
    assert!(
        err.to_string().contains("token budget exhausted"),
        "unexpected error: {err}"
    );
}

/// An agent step, an approval gate, then two more agent steps.
struct GatedTokenAgents;

impl WorkflowHandler for GatedTokenAgents {
    fn name(&self) -> &str {
        "gated-token-agents"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("step-0", agent_config()).await?;
            ctx.approval("gate", ApprovalConfig::new("Continue?"))
                .await?;
            ctx.agent("step-1", agent_config()).await?;
            ctx.agent("step-2", agent_config()).await?;
            Ok(())
        })
    }
}

/// Run [`GatedTokenAgents`] up to its gate and approve it.
async fn gated_token_run(engine: &Engine, store: &InMemoryStore) -> Uuid {
    let suspended = engine
        .run_handler("gated-token-agents", TriggerKind::Manual, json!({}))
        .await
        .expect("reaches the gate");
    assert_eq!(suspended.run.status.state, RunStatus::AwaitingApproval);
    approve_gate(store, suspended.run.id).await;
    suspended.run.id
}

#[tokio::test]
async fn tokens_spent_before_an_approval_count_after_the_resume() {
    // 700 tokens per step, 1500 allowed: 700 before the gate, 1400 after
    // step-1, step-2 crosses the budget only if the first 700 still count.
    let (dir, _guard) = fixtures_dir_with_tokens("token-approval", 500, 200);
    let store = Arc::new(InMemoryStore::new());
    let mut engine = token_engine(&store, &dir);
    engine.register(GatedTokenAgents).unwrap();

    let run_id = gated_token_run(&engine, &store).await;
    let err = engine
        .resume_run(run_id)
        .await
        .expect_err("the budget is exhausted after the resume");

    assert_token_rejection(&err);
    let run = stored_run(&store, run_id).await;
    assert_eq!(run.status.state, RunStatus::Cancelled);
    assert!(
        run.error
            .as_deref()
            .is_some_and(|e| e.contains("token budget exhausted")),
        "unexpected run error: {:?}",
        run.error
    );
}

#[tokio::test]
async fn a_run_without_guard_resumes_without_limits() {
    let (dir, _guard) = fixtures_dir_with_tokens("token-no-guard", 500, 200);
    let store = Arc::new(InMemoryStore::new());
    let mut engine = Engine::new(store.clone(), replay_provider(&dir));
    engine.register(GatedTokenAgents).unwrap();

    let run_id = gated_token_run(&engine, &store).await;
    let result = engine.resume_run(run_id).await.expect("resume completes");

    assert_eq!(result.run.status.state, RunStatus::Completed);
}

/// A shell step, an approval gate, then another shell step.
struct GatedSleeps;

impl WorkflowHandler for GatedSleeps {
    fn name(&self) -> &str {
        "gated-sleeps"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("before", ShellConfig::new("sleep 2")).await?;
            ctx.approval("gate", ApprovalConfig::new("Continue?"))
                .await?;
            ctx.shell("after", ShellConfig::new("sleep 2")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn time_spent_before_an_approval_counts_toward_the_workflow_timeout() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let guard = WorkflowGuardConfig::new().with_workflow_timeout_secs(3);
        let mut engine = engine_with_guard(store.clone(), guard);
        engine.register(GatedSleeps).unwrap();

        let suspended = engine
            .run_handler("gated-sleeps", TriggerKind::Manual, json!({}))
            .await
            .expect("reaches the gate within the timeout");
        assert_eq!(suspended.run.status.state, RunStatus::AwaitingApproval);
        let run_id = suspended.run.id;
        approve_gate(&store, run_id).await;

        let start = Instant::now();
        let err = engine
            .resume_run(run_id)
            .await
            .expect_err("the timeout is reached after the resume");

        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the timeout should interrupt the second sleep"
        );
        assert!(
            matches!(err, EngineError::WorkflowGuardRejected(_)),
            "expected WorkflowGuardRejected, got {err:?}"
        );
        assert!(err.to_string().contains("workflow timeout"));
        assert_eq!(
            stored_run(&store, run_id).await.status.state,
            RunStatus::Cancelled
        );
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn run_deadline_interrupts_a_running_step() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let deadline = Duration::from_secs(1);
        let guard = WorkflowGuardConfig::new().with_run_deadline(deadline);
        let mut engine = engine_with_guard(store.clone(), guard);
        engine.register(SlowWorkflow).unwrap();

        let start = Instant::now();
        let err = engine
            .run_handler("slow-workflow", TriggerKind::Manual, json!({}))
            .await
            .expect_err("should be interrupted at the deadline");

        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the deadline should interrupt the sleep"
        );
        assert!(
            matches!(err, EngineError::WorkflowGuardRejected(_)),
            "expected WorkflowGuardRejected, got {err:?}"
        );
        assert!(err.to_string().contains("run deadline reached"));
        let run = run_of(&store, "slow-workflow").await;
        assert_eq!(run.status.state, RunStatus::Cancelled);
    })
    .await
    .expect("test timed out");
}

/// Runs two slow shell steps in one parallel wave.
struct SlowParallelWorkflow;

impl WorkflowHandler for SlowParallelWorkflow {
    fn name(&self) -> &str {
        "slow-parallel-workflow"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.parallel(
                vec![
                    ("slow-a", StepConfig::Shell(ShellConfig::new("sleep 10"))),
                    ("slow-b", StepConfig::Shell(ShellConfig::new("sleep 10"))),
                ],
                true,
            )
            .await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn run_deadline_interrupts_a_parallel_wave() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let guard = WorkflowGuardConfig::new().with_run_deadline(Duration::from_secs(1));
        let mut engine = engine_with_guard(store.clone(), guard);
        engine.register(SlowParallelWorkflow).unwrap();

        let start = Instant::now();
        let err = engine
            .run_handler("slow-parallel-workflow", TriggerKind::Manual, json!({}))
            .await
            .expect_err("the wave should be interrupted at the deadline");

        assert!(
            start.elapsed() < Duration::from_secs(5),
            "the deadline should interrupt the parallel steps"
        );
        assert!(
            matches!(err, EngineError::WorkflowGuardRejected(_)),
            "expected WorkflowGuardRejected, got {err:?}"
        );
        assert!(err.to_string().contains("run deadline reached"));
        let run = run_of(&store, "slow-parallel-workflow").await;
        assert_eq!(run.status.state, RunStatus::Cancelled);
    })
    .await
    .expect("test timed out");
}

/// Waits for [`Release`] for an hour.
struct SignalWaiter;

impl WorkflowHandler for SignalWaiter {
    fn name(&self) -> &str {
        "signal-waiter"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.wait_for_signal::<Release>("wait", "k", Duration::from_secs(3600))
                .await?;
            Ok(())
        })
    }
}

/// An approval gate alone, run on its own or as a sub-workflow.
struct GateOnly;

impl TypedWorkflow for GateOnly {
    type Input = ();
}

impl WorkflowHandler for GateOnly {
    fn name(&self) -> &str {
        "gate-only"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.approval("gate", ApprovalConfig::new("Continue?"))
                .await?;
            Ok(())
        })
    }
}

/// An engine with `guard` and `handler`, ready for a [`RunWaker`].
fn shared_engine(
    store: Arc<InMemoryStore>,
    guard: WorkflowGuardConfig,
    handler: impl WorkflowHandler + 'static,
) -> Arc<Engine> {
    let engine = Engine::new(store, Arc::new(MissingAgentProvider));
    let mut engine = engine.with_guard_config(guard);
    engine.register(handler).unwrap();
    Arc::new(engine)
}

fn assert_stopped_at_deadline(run: &Run) {
    assert_eq!(run.status.state, RunStatus::Cancelled);
    assert!(
        run.error
            .as_deref()
            .is_some_and(|e| e.contains("run deadline reached")),
        "unexpected run error: {:?}",
        run.error
    );
}

#[tokio::test]
async fn run_deadline_stops_a_run_sleeping_on_a_signal() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let deadline = Duration::from_millis(300);
        let guard = WorkflowGuardConfig::new().with_run_deadline(deadline);
        let engine = shared_engine(store.clone(), guard, SignalWaiter);

        let suspended = engine
            .run_handler("signal-waiter", TriggerKind::Manual, json!({}))
            .await
            .expect("suspends on the signal");
        assert_eq!(suspended.run.status.state, RunStatus::Sleeping);
        let run_id = suspended.run.id;

        sleep(Duration::from_millis(600)).await;
        let woken = RunWaker::new(engine.clone()).tick().await.expect("tick");

        assert!(woken.is_empty(), "a run past its deadline is not woken");
        assert_stopped_at_deadline(&stored_run(&store, run_id).await);
        let signals = steps_of_kind(&store, run_id, StepKind::Signal).await;
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].status.state, StepStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn run_deadline_stops_a_run_awaiting_approval() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let deadline = Duration::from_millis(300);
        let guard = WorkflowGuardConfig::new().with_run_deadline(deadline);
        let engine = shared_engine(store.clone(), guard, GateOnly);

        let suspended = engine
            .run_handler("gate-only", TriggerKind::Manual, json!({}))
            .await
            .expect("reaches the gate");
        assert_eq!(suspended.run.status.state, RunStatus::AwaitingApproval);
        let run_id = suspended.run.id;

        sleep(Duration::from_millis(600)).await;
        RunWaker::new(engine.clone()).tick().await.expect("tick");

        assert_stopped_at_deadline(&stored_run(&store, run_id).await);
        let gates = steps_of_kind(&store, run_id, StepKind::Approval).await;
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].status.state, StepStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn a_run_without_deadline_keeps_waiting() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let engine = shared_engine(store.clone(), WorkflowGuardConfig::new(), GateOnly);

        let suspended = engine
            .run_handler("gate-only", TriggerKind::Manual, json!({}))
            .await
            .expect("reaches the gate");
        let run_id = suspended.run.id;

        sleep(Duration::from_millis(400)).await;
        RunWaker::new(engine.clone()).tick().await.expect("tick");

        assert_eq!(
            stored_run(&store, run_id).await.status.state,
            RunStatus::AwaitingApproval
        );
    })
    .await
    .expect("test timed out");
}

/// Runs [`GateOnly`] as a sub-workflow, under the engine's guard.
struct GateParent;

impl WorkflowHandler for GateParent {
    fn name(&self) -> &str {
        "gate-parent"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&GateOnly, ()).await?;
            Ok(())
        })
    }
}

/// Runs [`GateOnly`] as a sub-workflow, under its own guard without deadline.
struct UnboundedGateParent;

impl WorkflowHandler for UnboundedGateParent {
    fn name(&self) -> &str {
        "unbounded-gate-parent"
    }

    fn guard_config(&self) -> Option<WorkflowGuardConfig> {
        Some(WorkflowGuardConfig::new())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&GateOnly, ()).await?;
            Ok(())
        })
    }
}

/// The runs of `workflow` in `status`.
async fn runs_in_status(store: &InMemoryStore, workflow: &str, status: RunStatus) -> Vec<Run> {
    store
        .list_runs(
            RunFilter {
                workflow_name: Some(workflow.to_string()),
                status: Some(status),
                ..RunFilter::default()
            },
            1,
            100,
        )
        .await
        .expect("list runs")
        .items
}

#[tokio::test]
async fn run_deadline_sweep_stops_at_most_limit_runs() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let deadline = Duration::from_millis(300);
        let guard = WorkflowGuardConfig::new().with_run_deadline(deadline);
        let engine = shared_engine(store.clone(), guard, GateOnly);
        for _ in 0..3 {
            engine
                .run_handler("gate-only", TriggerKind::Manual, json!({}))
                .await
                .expect("reaches the gate");
        }

        sleep(Duration::from_millis(600)).await;
        let first = engine
            .cancel_runs_past_deadline(2)
            .await
            .expect("first sweep");
        assert_eq!(first.len(), 2);
        let waiting = runs_in_status(&store, "gate-only", RunStatus::AwaitingApproval).await;
        assert_eq!(waiting.len(), 1, "the limit leaves one run waiting");

        let second = engine
            .cancel_runs_past_deadline(2)
            .await
            .expect("second sweep");
        assert_eq!(second, vec![waiting[0].id]);
        for id in first.iter().chain(&second) {
            assert_stopped_at_deadline(&stored_run(&store, *id).await);
        }
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn run_deadline_sweep_stops_the_root_and_cancels_its_children() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let deadline = Duration::from_millis(300);
        let guard = WorkflowGuardConfig::new().with_run_deadline(deadline);
        let engine = Engine::new(store.clone(), Arc::new(MissingAgentProvider));
        let mut engine = engine.with_guard_config(guard);
        engine.register(GateParent).unwrap();
        engine.register(GateOnly).unwrap();

        let suspended = engine
            .run_handler("gate-parent", TriggerKind::Manual, json!({}))
            .await
            .expect("the child reaches its gate");
        assert_eq!(suspended.run.status.state, RunStatus::AwaitingApproval);
        let root_id = suspended.run.id;
        let child = run_of(&store, "gate-only").await;
        assert_eq!(child.status.state, RunStatus::AwaitingApproval);

        sleep(Duration::from_millis(600)).await;
        let stopped = engine.cancel_runs_past_deadline(10).await.expect("sweep");

        assert_eq!(
            stopped,
            vec![root_id],
            "the child is stopped through its root"
        );
        assert_stopped_at_deadline(&stored_run(&store, root_id).await);
        assert_stopped_at_deadline(&stored_run(&store, child.id).await);
        let gates = steps_of_kind(&store, child.id, StepKind::Approval).await;
        assert_eq!(gates.len(), 1);
        assert_eq!(gates[0].status.state, StepStatus::Failed);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn run_deadline_sweep_reaches_roots_behind_skipped_children() {
    timeout(TEST_TIMEOUT, async {
        let store = Arc::new(InMemoryStore::new());
        let deadline = Duration::from_millis(300);
        let guard = WorkflowGuardConfig::new().with_run_deadline(deadline);
        let engine = Engine::new(store.clone(), Arc::new(MissingAgentProvider));
        let mut engine = engine.with_guard_config(guard);
        engine.register(UnboundedGateParent).unwrap();
        engine.register(GateOnly).unwrap();

        // The standalone run is the oldest, so the newer child fills the first
        // page of `gate-only` runs.
        let standalone = engine
            .run_handler("gate-only", TriggerKind::Manual, json!({}))
            .await
            .expect("reaches the gate")
            .run
            .id;
        sleep(Duration::from_millis(20)).await;
        let parent = engine
            .run_handler("unbounded-gate-parent", TriggerKind::Manual, json!({}))
            .await
            .expect("the child reaches its gate")
            .run
            .id;

        sleep(Duration::from_millis(600)).await;
        let stopped = engine.cancel_runs_past_deadline(1).await.expect("sweep");

        assert_eq!(stopped, vec![standalone]);
        assert_stopped_at_deadline(&stored_run(&store, standalone).await);
        assert_eq!(
            stored_run(&store, parent).await.status.state,
            RunStatus::AwaitingApproval,
            "a root without deadline keeps waiting"
        );
        let children = runs_in_status(&store, "gate-only", RunStatus::AwaitingApproval).await;
        assert_eq!(children.len(), 1, "the child is never stopped on its own");
    })
    .await
    .expect("test timed out");
}

/// An agent step, then [`AskingChild`].
struct TokenRoot;

impl WorkflowHandler for TokenRoot {
    fn name(&self) -> &str {
        "token-root"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("root-step", agent_config()).await?;
            ctx.workflow(&AskingChild, ()).await?;
            Ok(())
        })
    }
}

/// Asks a human for a name, then runs two agent steps.
struct AskingChild;

impl TypedWorkflow for AskingChild {
    type Input = ();
}

impl WorkflowHandler for AskingChild {
    fn name(&self) -> &str {
        "asking-child"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let answer: Named = ctx
                .human_input("ask", HumanInputConfig::new("Name?"))
                .await?;
            assert_eq!(answer.name, "Ada");
            ctx.agent("child-1", agent_config()).await?;
            ctx.agent("child-2", agent_config()).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn tokens_spent_by_the_root_count_when_a_sub_workflow_child_resumes() {
    timeout(TEST_TIMEOUT, async {
        let (dir, _guard) = fixtures_dir_with_tokens("token-chain", 500, 200);
        let store = Arc::new(InMemoryStore::new());
        let mut engine = token_engine(&store, &dir);
        engine.register(TokenRoot).unwrap();
        engine.register(AskingChild).unwrap();

        let suspended = engine
            .run_handler("token-root", TriggerKind::Manual, json!({}))
            .await
            .expect("the child asks for a name");
        assert_eq!(suspended.run.status.state, RunStatus::AwaitingApproval);
        let root_id = suspended.run.id;

        let child = run_of(&store, "asking-child").await;
        answer_input(&store, child.id, "Ada").await;
        let err = engine
            .resume_run(child.id)
            .await
            .expect_err("the budget is exhausted in the resumed child");

        assert_token_rejection(&err);
        let root = stored_run(&store, root_id).await;
        assert_eq!(root.status.state, RunStatus::Cancelled);
        assert!(
            root.error
                .as_deref()
                .is_some_and(|e| e.contains("token budget exhausted")),
            "unexpected root error: {:?}",
            root.error
        );
    })
    .await
    .expect("test timed out");
}

/// An agent step, then a transient failure on the first attempt only, then
/// another agent step.
struct FlakyTokenAgents {
    attempts: AtomicU32,
}

impl WorkflowHandler for FlakyTokenAgents {
    fn name(&self) -> &str {
        "flaky-token-agents"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("step-0", agent_config()).await?;
            if self.attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(EngineError::Operation(OperationError::Http {
                    status: Some(503),
                    message: "upstream unavailable".to_string(),
                }));
            }
            ctx.agent("step-1", agent_config()).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn tokens_spent_by_a_failed_attempt_count_after_an_automatic_retry() {
    // Attempt 1 spends 700 then fails. Attempt 2 spends 700 on step-0 again
    // (1400 with the first attempt), step-1 crosses the 1500 budget.
    let (dir, _guard) = fixtures_dir_with_tokens("token-retry", 500, 200);
    let store = Arc::new(InMemoryStore::new());
    let mut engine = token_engine(&store, &dir);
    engine
        .register(FlakyTokenAgents {
            attempts: AtomicU32::new(0),
        })
        .unwrap();

    let run = engine
        .enqueue_handler("flaky-token-agents", TriggerKind::Manual, json!({}), 1)
        .await
        .expect("enqueue");
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("mark running");
    assert!(engine.execute_handler_run(run.id).await.is_err());
    let retrying = stored_run(&store, run.id).await;
    assert_eq!(retrying.status.state, RunStatus::Retrying);
    assert_eq!(retrying.retry_count, 1);

    fast_forward_backoff(&store, run.id).await;
    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("the budget is exhausted on the second attempt");

    assert_token_rejection(&err);
    assert_eq!(
        stored_run(&store, run.id).await.status.state,
        RunStatus::Cancelled
    );
}

/// An agent step, a signal wait, then two more agent steps.
struct SignalTokenAgents;

impl WorkflowHandler for SignalTokenAgents {
    fn name(&self) -> &str {
        "signal-token-agents"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("step-0", agent_config()).await?;
            let release = ctx
                .wait_for_signal::<Release>("wait", "k", Duration::from_secs(3600))
                .await?;
            assert_eq!(release.map(|r| r.version).as_deref(), Some("1.0"));
            ctx.agent("step-1", agent_config()).await?;
            ctx.agent("step-2", agent_config()).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn tokens_spent_before_a_signal_count_after_the_resume() {
    timeout(TEST_TIMEOUT, async {
        let (dir, _guard) = fixtures_dir_with_tokens("token-signal", 500, 200);
        let store = Arc::new(InMemoryStore::new());
        let mut engine = token_engine(&store, &dir);
        engine.register(SignalTokenAgents).unwrap();
        let engine = Arc::new(engine);

        let suspended = engine
            .run_handler("signal-token-agents", TriggerKind::Manual, json!({}))
            .await
            .expect("suspends on the signal");
        assert_eq!(suspended.run.status.state, RunStatus::Sleeping);
        let run_id = suspended.run.id;

        let release = Release {
            version: "1.0".to_string(),
        };
        let delivery = engine
            .send_signal(&release, "k", None)
            .await
            .expect("deliver");
        assert_eq!(delivery.resumed.len(), 1);

        wait_for_status(&store, run_id, RunStatus::Cancelled).await;
        let run = stored_run(&store, run_id).await;
        assert!(
            run.error
                .as_deref()
                .is_some_and(|e| e.contains("token budget exhausted")),
            "unexpected run error: {:?}",
            run.error
        );
    })
    .await
    .expect("test timed out");
}

/// An agent step, a human input, then two more agent steps.
struct InputTokenAgents;

impl WorkflowHandler for InputTokenAgents {
    fn name(&self) -> &str {
        "input-token-agents"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.agent("step-0", agent_config()).await?;
            let answer: Named = ctx
                .human_input("ask", HumanInputConfig::new("Name?"))
                .await?;
            assert_eq!(answer.name, "Ada");
            ctx.agent("step-1", agent_config()).await?;
            ctx.agent("step-2", agent_config()).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn tokens_spent_before_a_human_input_count_after_a_worker_requeue() {
    let (dir, _guard) = fixtures_dir_with_tokens("token-input", 500, 200);
    let store = Arc::new(InMemoryStore::new());
    let mut engine = token_engine(&store, &dir);
    engine.register(InputTokenAgents).unwrap();

    let run = engine
        .enqueue_handler("input-token-agents", TriggerKind::Manual, json!({}), 0)
        .await
        .expect("enqueue");
    store
        .update_run_status(run.id, RunStatus::Running)
        .await
        .expect("mark running");
    let suspended = engine
        .execute_handler_run(run.id)
        .await
        .expect("handler suspends on the input");
    assert_eq!(suspended.run.status.state, RunStatus::AwaitingApproval);

    answer_input(&store, run.id, "Ada").await;
    let err = engine
        .execute_handler_run(run.id)
        .await
        .expect_err("the budget is exhausted after the requeue");

    assert_token_rejection(&err);
    assert_eq!(
        stored_run(&store, run.id).await.status.state,
        RunStatus::Cancelled
    );
    let first_steps = store
        .list_steps(run.id)
        .await
        .expect("list steps")
        .into_iter()
        .filter(|s| s.name == "step-0")
        .count();
    assert_eq!(first_steps, 1, "the replayed step is not run again");
}
