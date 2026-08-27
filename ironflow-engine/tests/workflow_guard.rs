//! Integration tests for the [`WorkflowGuard`](ironflow_engine::guard).
//!
//! Tests exercise the guard through the [`Engine`] to verify that limits
//! are enforced end-to-end when running real (non-mocked) sub-workflows.

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::json;

use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::{RecordReplayProvider, hash_config};
use ironflow_engine::config::{AgentStepConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::error::EngineError;
use ironflow_engine::guard::{WORKFLOW_GUARD_REJECTED_CODE, WorkflowGuardConfig};
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, TriggerKind};

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
            ctx.workflow(&ChildWorkflow, json!({})).await?;
            Ok(())
        })
    }
}

struct ChildWorkflow;

impl WorkflowHandler for ChildWorkflow {
    fn name(&self) -> &str {
        "child"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&GrandchildWorkflow, json!({})).await?;
            Ok(())
        })
    }
}

struct GrandchildWorkflow;

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

impl WorkflowHandler for CycleA {
    fn name(&self) -> &str {
        "cycle-a"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&CycleB, json!({})).await?;
            Ok(())
        })
    }
}

struct CycleB;

impl WorkflowHandler for CycleB {
    fn name(&self) -> &str {
        "cycle-b"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.workflow(&CycleA, json!({})).await?;
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
                ctx.workflow(&GrandchildWorkflow, json!({})).await?;
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
            ctx.workflow(&ChildWorkflow, json!({})).await?;
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
            ctx.workflow(&ChildWorkflow, json!({})).await?;
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
        run.status.state,
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
        run.status.state,
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
        run.status.state,
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
        run.status.state,
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
            let steps: Vec<(&str, StepConfig)> = (0..self.n)
                .map(|_| ("agent", StepConfig::Agent(agent_config())))
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

    assert_eq!(run.status.state, RunStatus::Completed);
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

    let start = std::time::Instant::now();
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

    assert_eq!(run.status.state, RunStatus::Completed);
}
