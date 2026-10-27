# Parallel Execution

Ironflow supports running multiple steps in parallel within a workflow.

## Using ctx.parallel()

Pass a list of step configurations to `ctx.parallel()`. All steps run concurrently and the method returns when all complete:

```rust,ignore
{{#include ../../../../examples/ironflow-workflows/src/ci_pipeline.rs}}
```

## How it works

- All steps in a `parallel()` call start at the same time
- The method returns a `Vec<ParallelStepResult>` with outputs in the same order as the input;
  each `output` reads through the same typed accessors as a single step (`stdout()`,
  `status()`, `body()`, `is_success()`) and hands out the artifacts its step declared
  (`r.output.artifact("report.html")?`)
- If `fail_fast` is `true` (the second argument), the remaining steps are cancelled when one fails
- If `fail_fast` is `false`, all steps run to completion regardless of individual failures
- Every step of a wave needs its own name: a wave with two steps of the same name fails
  with `EngineError::StepConfig` before anything runs (a dry-run plan reports it too)
- When a run resumes (after an approval, a human input or a delay), the steps of the wave
  that already completed are replayed from the store; only the others run again
- `ctx.parallel()` takes shell, HTTP and agent steps. Sub-workflows run concurrently
  through `ctx.workflows_with()`, below

## Sub-workflows in parallel

`ctx.workflows_with(&Child, children)` starts one child run of `Child` per entry, each
with its own input and `WorkflowOptions`, and runs them concurrently:

```rust,ignore
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
    if let SubWorkflowOutcome::Completed(child) = outcome {
        println!("{} ended {}", child.run_id(), child.status());
    }
}
```

- One `Workflow` step per child, all at the same position; the outcomes come back in the
  order of the entries. Two entries with the same step name fail with
  `EngineError::StepConfig` before anything runs
- The child runs are created in list order, then run concurrently. A concurrency key
  already held, by an earlier sibling or by another run, gives a `Conflict` for that entry
- The call waits for every child. `allow_failure` is set per child; a failure it does not
  tolerate is returned once every sibling has finished and its step is recorded
- The cost cap of the root run is shared live by the whole tree: every agent step checks it
  against what the root, the siblings and their descendants spent and hold for their running
  steps, so a wave never takes the root past its cap. A child refused by the cap ends
  `Cancelled` with `RUN_BUDGET_EXCEEDED`
- The workflow guard counts each child as one invocation, and refuses a wave that would
  cross `max_fan_out` before anything is created
- A child that suspends leaves its step open while the others run to the end, then the
  parent suspends with it. On resume, finished children are replayed and suspended ones
  re-entered: no child run is created twice

When the children are agent roles picked by another agent, use `ctx.supervise` instead of
writing the loop: see
[A supervisor delegating to its team](../concepts/steps.md#a-supervisor-delegating-to-its-team).

## Conditional branching

Since workflows are Rust code, conditional logic is just `if`/`else`:

```rust,ignore
let results = ctx.parallel(steps, true).await?;
let all_passed = results.iter().all(|r| r.output.is_success());

if all_passed {
    ctx.shell("deploy", ShellConfig::new("echo 'Deploying'")).await?;
} else {
    ctx.shell("notify", ShellConfig::new("echo 'Tests failed'")).await?;
}
```

No special DSL for branching -- Rust control flow works directly.
