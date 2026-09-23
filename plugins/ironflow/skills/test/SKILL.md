---
name: test
description: Write an end-to-end test for an Ironflow workflow handler - real Engine, InMemoryStore, RecordReplayProvider fixtures, assertions on run status and step names. Loaded by the ironflow hub for the test verb.
user-invocable: false
---

# Ironflow workflow test

Black box: run the real handler against the in-memory store and assert on what was persisted. Two levels: `TestEngine` mocks the outside world (shell, HTTP, agent, approval) to exercise the handler's logic; the real `Engine` spawns real processes and replays recorded agent fixtures so the suite never spends tokens.

## Unit test with TestEngine

When every step of the handler is a shell, HTTP, agent or approval step, skip the engine boilerplate: `TestEngine` runs the real handler against an in-memory store with those steps mocked. No server, no worker, no Postgres, no process spawned.

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_engine::testing::{MockShellOutput, TestEngine};
use ironflow_store::models::{RunStatus, StepStatus};
use serde_json::json;

// In a real project: `use workflows::handlers::Deploy;`
struct Deploy;

impl WorkflowHandler for Deploy {
    fn name(&self) -> &str {
        "deploy"
    }
    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("cargo build")).await?;
            ctx.shell("ship", ShellConfig::new("./ship.sh")).await?;
            Ok(())
        })
    }
}

#[tokio::test]
async fn deploy_builds_then_ships() {
    let result = TestEngine::new()
        .with_handler(Deploy)
        .with_mock_shell(|cfg| match cfg.command.as_str() {
            "cargo build" => Ok(MockShellOutput::ok("compiled")),
            _ => Ok(MockShellOutput::ok("shipped")),
        })
        .run(json!({"environment": "staging"}))
        .await
        .expect("the harness ran the handler");

    assert_eq!(result.status(), RunStatus::Completed);
    assert_eq!(result.step_names(), vec!["build", "ship"]);
    assert_eq!(result.step("build").output()["stdout"], "compiled");
    assert_eq!(result.step("ship").status(), StepStatus::Completed);
}
```

Builders: `with_handler`, `with_mock_shell`, `with_mock_http`, `with_mock_agent`, `with_recorded_agent(dir)`, `with_mock_approval`, `with_agent_provider`, `with_decision_provider`. Then `run(payload)`, `run_workflow(name, payload)` or `resume(run_id)`.

A handler that fails is not an `Err`: `result.status()` is `RunStatus::Failed` and `result.error()` carries the message. A non-zero `MockShellOutput::failed(1, "boom")` fails the step like a real non-zero exit; a non-2xx `MockHttpResponse` is a normal output, like a real 500. Without `with_mock_approval`, a gate suspends the run (`RunStatus::AwaitingApproval`) and `resume(run_id)` continues it.

Not covered: `ctx.operation(...)` (pass a test-double `Operation` to the handler), `ctx.delay(...)` (still sleeps the run) and `ctx.decision(...)` (needs a real `DecisionProvider`).

## 1. Locate

```bash
grep -rn "impl WorkflowHandler for" --include=*.rs .      # the handler
grep -rl "pub fn handlers" --include=*.rs .              # the workflows crate
ls workflows/tests 2>/dev/null                            # existing tests and fixtures
```

Dev-dependencies the test needs, added once per crate (skip those already present):

```bash
cargo add -p workflows --dev ironflow-core ironflow-store serde_json
cargo add -p workflows --dev tokio --features macros,rt-multi-thread
```

## 2. Write the test

One file per handler in `workflows/tests/<handler>.rs`:

```rust,no_run
use std::sync::Arc;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::engine::Engine;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use ironflow_store::memory::InMemoryStore;
use ironflow_store::models::{RunStatus, StepStatus, TriggerKind};
use ironflow_store::store::Store;
use serde_json::json;

// In a real project: `use workflows::handlers;`
struct Deploy;

impl WorkflowHandler for Deploy {
    fn name(&self) -> &str {
        "deploy"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(Deploy)]
}

fn engine() -> Engine {
    let store: Arc<dyn Store> = Arc::new(InMemoryStore::new());
    // Replays `tests/fixtures/<hash>.json`; records with IRONFLOW_RECORD=1.
    let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::new(
        ClaudeCodeProvider::new(),
        "tests/fixtures",
    ));
    let mut engine = Engine::new(store, provider);
    for handler in handlers() {
        engine.register(handler).expect("handler names are unique");
    }
    engine
}

#[tokio::test]
async fn deploy_to_staging_completes() {
    let result = engine()
        .run_handler(
            "deploy",
            TriggerKind::Manual,
            json!({"git_ref": "main", "environment": "staging"}),
        )
        .await
        .expect("run completes");

    assert_eq!(result.run.status.state, RunStatus::Completed);
    let names: Vec<&str> = result.steps.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, vec!["build", "test", "lint", "deploy"]);
    assert!(result.steps.iter().all(|s| s.status == StepStatus::Completed));
}
```

Adapt: handler name, payload, expected step names in order. `result.steps` is in execution order; parallel steps appear in the order they were declared.

## 3. Fixtures for agent steps

First run records, later runs replay:

```bash
IRONFLOW_RECORD=1 cargo test -p workflows --test <handler>   # calls the provider, writes tests/fixtures/<hash>.json
cargo test -p workflows --test <handler>                     # replays, no tokens spent
```

The hash covers the prompt, system prompt and output schema, so a prompt change means re-recording. Commit `tests/fixtures/`. A missing fixture falls back to the real provider with a warning, which is how a stale suite silently starts costing money: check the test output for `fixture not found`.

## 4. Verify

```bash
cargo test -p workflows --test <handler>
```

Report the assertion list and whether a fixture was recorded.

## What the engine does with a failure

`run_handler` returns `Err(EngineError)` when the handler fails: a payload that does not deserialize, a shell step with a non-zero exit code, an HTTP transport error. Assert on the error text with `err.to_string()`. A step configured with `allow_failure()` does not fail the run; the run ends with `RunStatus::Warning` instead.

## Shell steps in tests

They spawn real processes. Keep commands portable (`echo`, `true`, `sh -c`) or gate the test on the tool with a runtime check, never by mocking the step.

Use the real `Engine` when the test must exercise real commands; use `TestEngine` when it must exercise the handler's logic.
