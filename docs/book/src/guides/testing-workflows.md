# Testing Workflows

`ironflow_engine::testing::TestEngine` runs a `WorkflowHandler` against an
in-memory store with mocked steps. The run, the steps, the FSM transitions and
the persistence are the production ones -- only the outside world is swapped
out.

What it replaces:

| Production | Under `TestEngine` |
|------------|--------------------|
| API server | nothing to start; the run executes inline |
| Background worker | nothing to start; `run()` returns once the run is finished |
| Postgres | `InMemoryStore` |
| `sh -c <command>` | a closure |
| An HTTP request | a closure |
| The Claude CLI | a closure, or a recorded fixture |
| A human clicking *Approve* | an `ApprovalOutcome` |

## A first test

```rust,ignore
use ironflow_engine::prelude::*;
use ironflow_engine::testing::{MockShellOutput, TestEngine};
use ironflow_store::models::{RunStatus, StepStatus};
use serde_json::json;

use crate::handlers::Deploy;

#[tokio::test]
async fn deploy_runs_build_then_ship() {
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
    assert_eq!(result.step("build").step_output().stdout(), "compiled");
    assert_eq!(result.step("ship").status(), StepStatus::Completed);
}
```

A handler that fails is **not** an `Err`: the returned `TestResult` carries
`RunStatus::Failed` and the message in `error()`. Only wiring failures -- no
handler registered, two handlers sharing a name, a store rejection -- come back
as `Err`.

## Building the harness

| Method | What it does |
|--------|--------------|
| `with_handler(handler)` | Registers a handler. The first one is what `run()` executes. |
| `with_mock_shell(f)` | Answers every shell step from `f(&ShellConfig)`. |
| `with_mock_http(f)` | Answers every HTTP step from `f(&HttpConfig)`. |
| `with_mock_agent(f)` | Answers every agent step from `f(&AgentConfig)`. |
| `with_recorded_agent(dir)` | Replays agent fixtures from `dir`. |
| `with_agent_provider(p)` | Uses an arbitrary `AgentProvider`. |
| `with_decision_provider(p)` | Wires a `DecisionProvider` for `ctx.decision(...)`. |
| `with_mock_approval(outcome)` | Resolves every approval gate with `outcome`. |
| `store()` | The `InMemoryStore`, for assertions the accessors do not cover. |

Every `with_*` method panics if called after the first run: the engine is built
once, so a later change would be silently ignored.

Then run:

| Method | What it does |
|--------|--------------|
| `run(payload)` | Runs the first registered handler. |
| `run_workflow(name, payload)` | Runs a specific registered handler. |
| `resume(run_id)` | Continues a run suspended on an approval gate. |

## Asserting on the result

`TestResult` reads the run and its steps back from the store, so an assertion
sees exactly what the API and the dashboard would serve.

| Accessor | Returns |
|----------|---------|
| `status()` | The `RunStatus` the run finished in. |
| `is_completed()` | Whether that status is `Completed`. |
| `error()` | Why the run stopped, if it did not complete. |
| `steps()` | Every persisted step, ordered by position. |
| `step_names()` | Those steps' names, in the same order. |
| `step(name)` | The first step with that name; panics when there is none. |
| `try_step(name)` | The same, as an `Option`. |
| `output()` | The last step's output. |
| `duration()`, `cost_usd()` | The run totals. |
| `run_id()`, `run()` | The run identity and the raw record. |
| `step_results()` | Per-step metrics, empty when the run failed. |

Each `TestStep` exposes `name()`, `kind()`, `status()`, `step_output()`,
`output()`, `input()`, `error()`, `duration()`, `cost_usd()`, `is_completed()`,
`is_error_handler()` and `raw()`. `step_output()` reads the persisted output
through the typed `StepOutput` accessors (`stdout()`, `status()`, `body()`,
`text()`, `json::<T>()`); `output()` is the raw JSON.

Steps of a parallel wave share a position and a handler may reuse a name:
disambiguate those with `steps()` rather than `step(name)`.

## Shell and HTTP parity

The mocks reproduce the asymmetry of the real executors, so `allow_failure`,
step retries and run failure behave exactly as in production:

* A `MockShellOutput` with a non-zero `exit_code` is an **error**, like a real
  non-zero exit. Use `MockShellOutput::failed(1, "boom")`.
* A `MockHttpResponse` with a non-2xx `status` is a normal **output**, like a
  real 500 response. Return `Err(OperationError::Http { status: None, .. })`
  from the closure to simulate a transport failure instead.

```rust,ignore
use ironflow_core::error::OperationError;
use ironflow_engine::testing::{MockHttpResponse, TestEngine};
use serde_json::json;

let harness = TestEngine::new()
    .with_handler(Fetch)
    // A 404 the handler is expected to deal with.
    .with_mock_http(|cfg| {
        if cfg.url.ends_with("/missing") {
            Ok(MockHttpResponse::json(404, &json!({"error": "not found"})))
        } else {
            Ok(MockHttpResponse::ok(&json!({"id": 7})))
        }
    });
```

## Approval gates

Two ways to test a gated handler:

```rust,ignore
use ironflow_engine::testing::{ApprovalOutcome, TestEngine};

// 1. Resolve the gate inline and assert on the whole run.
let approved = TestEngine::new()
    .with_handler(GatedDeploy)
    .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ok")))
    .with_mock_approval(ApprovalOutcome::Approved)
    .run(json!({}))
    .await?;
assert_eq!(approved.status(), RunStatus::Completed);

// A rejection fails the run with EngineError::ApprovalRejected.
let rejected = TestEngine::new()
    .with_handler(GatedDeploy)
    .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ok")))
    .with_mock_approval(ApprovalOutcome::reject("budget freeze"))
    .run(json!({}))
    .await?;
assert_eq!(rejected.step("gate").status(), StepStatus::Rejected);
```

```rust,ignore
// 2. Without a mock, the gate suspends the run, the way production does.
let mut harness = TestEngine::new()
    .with_handler(GatedDeploy)
    .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ok")));

let suspended = harness.run(json!({})).await?;
assert_eq!(suspended.status(), RunStatus::AwaitingApproval);

let resumed = harness.resume(suspended.run_id()).await?;
assert_eq!(resumed.status(), RunStatus::Completed);
```

## Agent fixtures

`with_recorded_agent(dir)` replays fixtures written by `RecordReplayProvider`.
The argument is the **directory**: each fixture is keyed by a hash of the
`AgentConfig` and stored as `<hash>.json` inside it. A missing fixture fails the
step instead of falling back to the real Claude CLI, so a stale suite never
silently starts spending tokens.

```rust,ignore
let result = TestEngine::new()
    .with_handler(Review)
    .with_recorded_agent("tests/fixtures")
    .run(json!({}))
    .await?;
```

To record, pass a recording provider through the escape hatch:

```rust,ignore
use std::sync::Arc;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::record_replay::RecordReplayProvider;

let provider: Arc<dyn AgentProvider> = Arc::new(RecordReplayProvider::record(
    ClaudeCodeProvider::new(),
    "tests/fixtures",
));
let result = TestEngine::new()
    .with_handler(Review)
    .with_agent_provider(provider)
    .run(json!({}))
    .await?;
```

With no agent backend configured at all, an agent step fails with a message
naming the three constructors -- a forgotten mock is a loud failure, not a
network call.

## Parallel waves, error handlers and sub-workflows

The mocks apply to the steps inside them: a step of a `ctx.parallel(...)` wave,
a step fired by `ctx.on_error(...)`, and every step of a child run started with
`ctx.workflow(...)` all go through the same interceptor. Register both handlers
and drive the parent:

```rust,ignore
let mut harness = TestEngine::new()
    .with_handler(Parent)
    .with_handler(Child)
    .with_mock_shell(|_cfg| Ok(MockShellOutput::ok("ok")));
let store = harness.store();

let result = harness.run_workflow("parent", json!({})).await?;
// A workflow step stores a `SubWorkflowOutput`: read it back typed.
let child: SubWorkflowOutput = result.step("child").step_output().json()?;
let child_steps = store.list_steps(child.run_id()).await?;
```

## Limitations

* Custom operations (`ctx.operation(...)`) are not intercepted. Mock one by
  passing a test-double `Operation` to the handler.
* `ctx.delay(...)` is not intercepted: a non-zero delay still suspends the run
  with `RunStatus::Sleeping`.
* `ctx.decision(...)` needs a real `DecisionProvider`, wired with
  `with_decision_provider`.

Use the real `Engine` when the test must exercise real commands, real requests
or a real agent; use `TestEngine` when it must exercise the handler's logic.
