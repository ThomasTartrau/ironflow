---
name: workflow
description: Write an Ironflow WorkflowHandler - typed input schema, steps (shell, http, agent, approval, sub-workflow, parallel), registration in handlers(). Loaded by the ironflow hub for the workflow verb.
user-invocable: false
---

# Ironflow workflow

A workflow is a struct implementing `WorkflowHandler`. Control flow is plain Rust. Each `ctx.*` call is a persisted step with its own status, duration and cost.

## 1. Gather the shape

Ask only what the code cannot guess, in one AskUserQuestion call:

- workflow name (kebab-case, unique) and one-line purpose
- input fields (name, type, required or default)
- the steps in order, and which of them run in parallel
- whether a human approval gate sits somewhere, and whether an agent step needs a budget above `0.10` USD

Then locate the workflows crate: `grep -rl "pub fn handlers" --include=*.rs .`

## 2. Write the handler

One file per handler in the workflows crate, `snake_case.rs`. Template:

```rust
use ironflow_engine::config::{ApprovalConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

/// Input payload of the `deploy` workflow. The dashboard renders a form from it.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeployInput {
    /// Git ref to deploy.
    pub git_ref: String,
    /// Target environment.
    #[serde(default = "default_env")]
    pub environment: String,
}

fn default_env() -> String {
    "staging".to_string()
}

/// Builds, tests in parallel, waits for a human, deploys.
pub struct Deploy;

impl WorkflowHandler for Deploy {
    fn name(&self) -> &str {
        "deploy"
    }

    fn description(&self) -> &str {
        "Build, run checks in parallel, wait for approval, deploy."
    }

    fn category(&self) -> Option<&str> {
        Some("ops")
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<DeployInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: DeployInput = ctx.input().await?;

            ctx.shell(
                "build",
                ShellConfig::new("cargo build --release").env("GIT_REF", &input.git_ref),
            )
            .await?;

            // Fan out. `true` = fail fast on the first error.
            let checks = ctx
                .parallel(
                    vec![
                        ("test", StepConfig::Shell(ShellConfig::new("cargo test"))),
                        ("lint", StepConfig::Shell(ShellConfig::new("cargo clippy"))),
                    ],
                    true,
                )
                .await?;
            if !checks.iter().all(|c| c.output.is_success()) {
                return Ok(());
            }

            if input.environment == "production" {
                // The run suspends here. After approval the handler is replayed from
                // the top: completed steps come back from cache, this code runs again.
                ctx.approval(
                    "approve-production",
                    ApprovalConfig::new("Ship to production?").with_timeout_seconds(3600),
                )
                .await?;
            }

            ctx.shell(
                "deploy",
                ShellConfig::new("./deploy.sh \"$ENVIRONMENT\"").env("ENVIRONMENT", &input.environment),
            )
            .await?;

            Ok(())
        })
    }
}
```

Then add the source display, which needs the real file name:

```rust,ignore
    fn source_code(&self) -> Option<&str> {
        Some(include_str!("deploy.rs"))
    }
```

Step catalogue with every config builder: `references/steps.md`. Read it before writing an agent, http, sub-workflow, secret or artifact step.

## 3. Register

In the workflows crate `lib.rs`: `mod deploy;`, `pub use deploy::{Deploy, DeployInput};`, and one line in `handlers()`:

```rust,ignore
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![Box::new(Hello), Box::new(Deploy)]
}
```

That list feeds both the server and the worker. Nothing else to touch.

## 4. Verify

```bash
cargo build -p workflows
cargo test -p workflows
```

Then offer, in one sentence, the workflow reviewer (`/ironflow review`) and the test skill (`/ironflow test <name>`). Do not run either without an answer.

## Rules that are not obvious

- **Step names are cache keys.** Stable, unique within the run, no timestamps or random ids. In a loop, suffix with the loop index.
- **Approval replays the handler.** Anything that is not a `ctx.*` step runs again after approval. Keep side effects inside steps. Details and a safe pattern: `references/approval-replay.md`.
- **Agent steps: tools or structured output, not both.** `AgentStepConfig::new(prompt).allow_tool("Read")` and `.output::<T>()` are mutually exclusive by type.
- **Budget.** Claude Code's system cache alone costs about `0.04` USD, so `max_budget_usd` below `0.10` fails. Put a `default_max_cost_usd` on the handler when it contains an agent step.
- **User data goes through `env`, never into the command string.** `ShellConfig::new("echo \"$X\"").env("X", value)`.
- **A failing step fails the run.** No hidden retries. `allow_failure()` on a step config lets the run continue with status `Warning`; `retry_policy(RetryPolicy::...)` opts into retries explicitly.
- **`ctx.input::<T>()` errors are `EngineError::Serialization`.** The engine also validates the payload against `input_schema()` before creating a run through the API, so the handler rarely sees a bad payload.
