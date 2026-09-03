# Approval gates replay the handler

When `ctx.approval()` is reached the run stops with status `AwaitingApproval` and the
handler future is dropped. On approval the engine **runs `execute()` again from the first
line**. Every step already completed returns its cached output without executing; the
approval step is skipped; execution continues with the first step that has no cache entry.

Consequence: any code that is not a `ctx.*` step runs twice.

## What breaks

```rust,no_run
use ironflow_engine::config::{ApprovalConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use serde_json::json;

async fn broken(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    // Runs twice: once before the gate, once after approval.
    let client = reqwest::Client::new();
    client
        .post("https://hooks.example.com/started")
        .json(&json!({"run": ctx.run_id()}))
        .send()
        .await
        .map_err(|e| EngineError::StepConfig(e.to_string()))?;

    // Different name on replay: the cache misses and the step runs again.
    let build_name = format!("build-{}", chrono::Utc::now().timestamp());
    ctx.shell(&build_name, ShellConfig::new("cargo build")).await?;

    ctx.approval("gate", ApprovalConfig::new("Continue?")).await?;
    Ok(())
}
```

## The safe pattern

Everything with a side effect is a step, and step names are constants.

```rust,no_run
use ironflow_engine::config::{ApprovalConfig, HttpConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use serde_json::json;

async fn safe(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    // A step: cached on replay, executed exactly once.
    ctx.http(
        "notify-started",
        HttpConfig::post("https://hooks.example.com/started")
            .json(json!({"run": ctx.run_id()})),
    )
    .await?;

    ctx.shell("build", ShellConfig::new("cargo build")).await?;

    ctx.approval("gate", ApprovalConfig::new("Continue?")).await?;

    ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?;
    Ok(())
}
```

Pure computation between steps (formatting a prompt from a cached output, an `if` on a
cached exit code) is fine: it is deterministic and cheap.

## Names inside loops

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext, hosts: &[String]) -> Result<(), EngineError> {
    for (i, host) in hosts.iter().enumerate() {
        // Index-based, stable across replays as long as the input is the same.
        ctx.shell(
            &format!("deploy-{i}"),
            ShellConfig::new("./deploy.sh \"$HOST\"").env("HOST", host),
        )
        .await?;
    }
    Ok(())
}
```

Do not derive names from values that can change between attempts (time, random ids,
unordered collections).

## Rejection and timeout

Rejection makes the approval step `Failed` and the run `Failed`. `with_timeout_seconds`
expires the gate the same way. Neither resumes the handler, so there is no "else" branch to
write: put the rollback in an `on_error` handler registered before the gate.
