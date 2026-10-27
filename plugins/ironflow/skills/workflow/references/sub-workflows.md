# Sub-workflows

A workflow can run another workflow as a child run: one child at a time with
`ctx.workflow` / `ctx.workflow_with`, or several at once with `ctx.workflows_with`. Each
snippet compiles against the current Ironflow release (CI checks them).

## Typed child

A child called as a sub-workflow declares its input type with `TypedWorkflow`; the parent
can then only pass that type.

```rust,no_run
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{
    HandlerFuture, TypedWorkflow, WorkflowHandler, sub_workflow_names,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Serialize, Deserialize, JsonSchema)]
struct CollectInput {
    scope: String,
}

/// What `Collect` hands back to its parent.
#[derive(Serialize, Deserialize)]
struct CollectReport {
    disks: Vec<String>,
}

struct Collect;

impl WorkflowHandler for Collect {
    fn name(&self) -> &str {
        "collect"
    }
    fn input_schema(&self) -> Option<Value> {
        Self::typed_input_schema()
    }
    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Persisted on the run when it ends, even if the handler fails after.
            ctx.set_output(&CollectReport { disks: vec!["sda".to_string()] })?;
            Ok(())
        })
    }
}

impl TypedWorkflow for Collect {
    type Input = CollectInput;
}

struct Report;

impl WorkflowHandler for Report {
    fn name(&self) -> &str {
        "report"
    }
    // The handlers themselves, not their names: a typo does not compile.
    fn sub_workflows(&self) -> Vec<String> {
        sub_workflow_names(&[&Collect])
    }
    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            // Runs `Collect` as a child run. Its cost is added to the parent.
            let child = ctx
                .workflow(&Collect, CollectInput { scope: "system".to_string() })
                .await?;
            // `None` when the child set no output; an error when it is not a `CollectReport`.
            if let Some(report) = child.output::<CollectReport>()? {
                ctx.set_output(&report.disks)?;
            }
            Ok(())
        })
    }
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    ctx.workflow(&Collect, CollectInput { scope: "disk".to_string() })
        .await?;
    Ok(())
}
```

Pass `WorkflowOptions::new().allow_failure()` to `ctx.workflow_with(&Child, input, options)` to
tolerate a failed child: the step completes with a `Completed` outcome whose `status()` is `Failed` (or `Cancelled`),
`error()` carries the message and the parent ends as `Warning`. A child cancelled through
`POST /runs/:id/cancel` gives the same `Cancelled` outcome; without `allow_failure` the step
fails with `EngineError::ChildRunCancelled`, which is never retried. A suspension is never
tolerated, and catching the error of a plain `ctx.workflow` is not a substitute: the failed step
is not completed, so a resume runs the child again.

```rust
use ironflow_engine::config::WorkflowOptions;
use ironflow_store::entities::RunStatus;
# use ironflow_engine::context::WorkflowContext;
# use ironflow_engine::error::EngineError;
# use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
# use serde::{Deserialize, Serialize};
# #[derive(Serialize, Deserialize)]
# struct CollectInput { scope: String }
# struct Collect;
# impl WorkflowHandler for Collect {
#     fn name(&self) -> &str { "collect" }
#     fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
#         Box::pin(async move { Ok(()) })
#     }
# }
# impl TypedWorkflow for Collect { type Input = CollectInput; }

async fn tolerant(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let outcome = ctx
        .workflow_with(&Collect, CollectInput { scope: "disk".to_string() }, WorkflowOptions::new().allow_failure())
        .await?;
    if let Some(child) = outcome.output().filter(|c| c.status() == RunStatus::Failed) {
        println!("collect failed: {:?}", child.error());
    }
    Ok(())
}
```

`child.run_id()` is a `Uuid` (nil while planning). A child never sees the parent's
artifacts; pass what it needs in its input.

`child.output::<T>()` reads what the child passed to `ctx.set_output`: `Ok(None)` when it set
nothing, `Err(EngineError::Serialization)` when the value is not a `T`. The value is recorded
in the parent's `Workflow` step, so a resumed parent reads the same one. A child tolerated
with `allow_failure` keeps the output it set before failing. `set_output` does nothing while
planning; the last call wins.

## Exclusive child

To start a child only when no other active run handles the same subject (an issue, a
branch), pass a concurrency key through `ctx.workflow_with`. A conflict is not an error:
no child is created and the step completes with the run that holds the key. The outcome
is replayed as recorded when the parent resumes.

```rust,no_run
use ironflow_engine::config::WorkflowOptions;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::executor::SubWorkflowOutcome;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct FixInput {
    issue: u64,
}

struct FixIssue;

impl WorkflowHandler for FixIssue {
    fn name(&self) -> &str {
        "fix-issue"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

impl TypedWorkflow for FixIssue {
    type Input = FixInput;
}

async fn example(ctx: &mut WorkflowContext, issue: u64) -> Result<(), EngineError> {
    let options = WorkflowOptions::new().concurrency_key(format!("issue:{issue}"));
    match ctx.workflow_with(&FixIssue, FixInput { issue }, options).await? {
        SubWorkflowOutcome::Completed(child) => println!("fixed in run {}", child.run_id()),
        SubWorkflowOutcome::Conflict(conflict) => {
            println!("issue {issue} already handled by run {}", conflict.run_id());
        }
    }
    Ok(())
}
```

The same key on `POST /api/v1/runs` (`concurrency_key` in the body) answers
`409 CONCURRENCY_CONFLICT` instead.

To queue runs rather than refuse them, give a root run concurrency groups on
`POST /api/v1/runs`: `"concurrency_limits": [{"group": "repo:acme", "limit": 2}]` (or
`EnqueueOptions::concurrency_limits` on the engine, `--concurrency-limit repo:acme=2` in the
CLI). The run stays pending until fewer than `limit` runs of each of its groups are
running. A child started with `ctx.workflow` runs inside the slot of its parent and joins no
group.

To order the queue, give a run a priority from -100 to 100 (default 0, or the handler's
`fn priority(&self) -> i16`): `"priority": 50` on `POST /api/v1/runs`,
`EnqueueOptions { priority: Some(50), .. }` on the engine, `--priority 50` in the CLI,
`WorkflowOptions::new().priority(50)` for a child. Workers pick the highest priority first,
then the oldest. A running run is never preempted and a waiting one is never aged: a steady
flow of higher priorities can delay a low one indefinitely.

## Named, labelled and parallel children

The step of a child is named after its handler. When the same child runs more than once
in a run (a loop, a wave), name each step with `WorkflowOptions::step_name`: like every
step name it is a replay key, so keep it stable and unique. `label(key, value)` (or
`labels([..])`) adds labels to the child run, over the labels it inherits from its parent;
a key under `ironflow.io/` belongs to the engine and panics, as does a blank step name.

`ctx.workflows_with(&Child, children)` runs one child per entry, concurrently, each with
its own input and options, and returns the outcomes in the order of the entries:

```rust,no_run
use ironflow_engine::config::WorkflowOptions;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::executor::SubWorkflowOutcome;
use ironflow_engine::handler::{HandlerFuture, TypedWorkflow, WorkflowHandler};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct DelegateInput {
    agent: String,
}

struct Delegate;

impl WorkflowHandler for Delegate {
    fn name(&self) -> &str {
        "delegate"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

impl TypedWorkflow for Delegate {
    type Input = DelegateInput;
}

async fn delegate_tour(
    ctx: &mut WorkflowContext,
    tour: u32,
    agents: &[String],
) -> Result<(), EngineError> {
    let wave = agents
        .iter()
        .map(|agent| {
            let options = WorkflowOptions::new()
                .step_name(format!("delegate-{tour}-{agent}"))
                .label("team.example/agent", agent.as_str())
                .allow_failure();
            (DelegateInput { agent: agent.clone() }, options)
        })
        .collect();
    for outcome in ctx.workflows_with(&Delegate, wave).await? {
        match outcome {
            SubWorkflowOutcome::Completed(child) => {
                println!("{} ended {}", child.run_id(), child.status());
            }
            SubWorkflowOutcome::Conflict(conflict) => {
                println!("already handled by run {}", conflict.run_id());
            }
        }
    }
    Ok(())
}
```

- Two entries with the same step name fail with `EngineError::StepConfig` before any
  child is created.
- The child runs are created in list order, then run concurrently. A concurrency key
  already held, by an earlier sibling or by another run, gives a `Conflict` for that entry.
- The call waits for every child. `allow_failure` is set per child; a failure it does not
  tolerate is returned once every sibling has finished, with every step recorded.
- The cost cap of the root run is shared live by the whole tree: a wave never takes the
  root past it. A child refused by the cap ends `Cancelled` with `RUN_BUDGET_EXCEEDED`.
- The workflow guard counts each child as one invocation and refuses a wave that would
  cross `max_fan_out` before anything is created.
- `ctx.workflow_with` is unchanged: one child, sequential.
- When an agent picks the children (a supervisor handing tasks to other roles), do not
  write that loop around a wave: use `ctx.supervise`, see `supervisor.md`.

## Suspension

A child may suspend (approval, human input, signal wait, delay): the parent and every
ancestor suspend with it, durably. Resolve the child run itself, by its id or by the
`ironflow.io/parent-run-id` label (`PARENT_RUN_ID_LABEL` from `ironflow_engine::context`):
resuming it resumes the root run, which replays its completed steps and re-enters the
same child run. A child already completed is not run again.

In a wave, a child that suspends leaves its step open while its siblings run to the end,
then the parent suspends. Each suspended child publishes its own approval or signal event
and is resolved on its own; a resume that arrives while the root is already running waits
for it instead of failing. Finished children are replayed, suspended ones re-entered: no
child run is created twice.

Inside the child, `ctx.run_id()` is the child run and `ctx.root_run_id()` the top-level
run that started the chain (both equal at the top). A pod you create yourself in a
step (`PodRun`, `JobRun` of `ironflow-ops-k8s`) carries both, as
`LABEL_RUN_ID` and `LABEL_ROOT_RUN_ID` from `ironflow_core::provider`: a retry of the
top-level run then deletes what a dead attempt left running.
