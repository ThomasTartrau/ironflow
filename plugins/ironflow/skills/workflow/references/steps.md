# Step catalogue

Every method on `WorkflowContext`, with its config builder. Each snippet compiles against
the current Ironflow release (CI checks them).

## Shell

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let out = ctx
        .shell(
            "build",
            ShellConfig::new("cargo build --release")
                .dir("/app")
                .env("RUSTFLAGS", "-D warnings")
                .timeout_secs(600),
        )
        .await?;
    // Typed accessors on the step output.
    let _code = out.exit_code();
    let _stdout = out.stdout();
    let _ok = out.is_success();
    Ok(())
}
```

Other builders: `clean_env()` (start from an empty environment), `allow_failure()`,
`retry_policy(RetryPolicy)`, `output("target/*.log")` and `input("build", "report.html")`
for artifacts (below).

## HTTP

Non-2xx statuses are not errors: check `is_success()` or `status()`.

```rust,no_run
use ironflow_engine::config::HttpConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
struct Health {
    ok: bool,
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let resp = ctx
        .http(
            "notify",
            HttpConfig::post("https://api.example.com/notify")
                .header("Authorization", "Bearer token")
                .json(json!({"status": "deployed"}))
                .timeout_secs(30),
        )
        .await?;
    if !resp.is_success() {
        return Err(EngineError::StepConfig(format!("notify answered {:?}", resp.status())));
    }
    // The body is a string; parse it when it carries JSON.
    let _health: Health = serde_json::from_str(resp.body())?;
    Ok(())
}
```

`HttpConfig::{get, post, put, patch, delete}`, plus `allow_failure()` and
`retry_policy(...)`.

## Agent

Either tools or a structured output, never both (enforced by the type state).

```rust,no_run
use ironflow_core::operations::agent::Model;
use ironflow_engine::config::AgentStepConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct Review {
    score: u8,
    summary: String,
}

async fn example(ctx: &mut WorkflowContext, diff: &str) -> Result<(), EngineError> {
    // Structured output: the provider is constrained to the schema of `Review`.
    let out = ctx
        .agent(
            "review",
            AgentStepConfig::new(&format!("Review this diff:\n{diff}"))
                .system_prompt("You are a senior Rust reviewer.")
                .model(Model::SONNET)
                .max_turns(3)
                .max_budget_usd(0.25)
                .output::<Review>(),
        )
        .await?;
    let review: Review = out.json()?;
    let _ = (review.score, review.summary);

    // Tools: the agent can act, and answers in free text.
    let explore = ctx
        .agent(
            "explore",
            AgentStepConfig::new("List the top-level files and summarise the project.")
                .allow_tool("Bash")
                .allow_tool("Read")
                .max_turns(6)
                .max_budget_usd(0.50)
                .verbose(true),
        )
        .await?;
    let _text = explore.output.as_str().unwrap_or_default();
    Ok(())
}
```

`Model::SONNET`, `Model::OPUS`, `Model::HAIKU` are aliases resolved by the provider;
pass a full model id string for a pinned version. `verbose(true)` records the tool
timeline shown in the dashboard.

## Approval

```rust,no_run
use ironflow_engine::config::ApprovalConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    ctx.approval(
        "approve-production",
        ApprovalConfig::new("Staging looks good. Deploy to production?")
            .with_timeout_seconds(3600),
    )
    .await?;
    Ok(())
}
```

The run moves to `AwaitingApproval`. `POST /api/v1/runs/{id}/approve` (dashboard, CLI,
MCP) resumes it: the handler is replayed from the top. Rejection fails the run. Read
`approval-replay.md` before putting code between steps around a gate.

## Sub-workflow

```rust,no_run
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};
use serde_json::json;

struct Collect;

impl WorkflowHandler for Collect {
    fn name(&self) -> &str {
        "collect"
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
    }
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    // Runs `Collect` as a child run. Its cost is added to the parent.
    let child = ctx.workflow(&Collect, json!({"scope": "system"})).await?;
    let _child_run_id = child.output.get("run_id").and_then(|v| v.as_str());
    Ok(())
}
```

Declare it in the parent's `sub_workflows()` so the dashboard draws the call graph. A child
never sees the parent's artifacts; pass what it needs in the payload.

## Parallel

```rust,no_run
use ironflow_engine::config::{HttpConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    // `true`: stop at the first failure. `false`: run everything, then report.
    let results = ctx
        .parallel(
            vec![
                ("test", StepConfig::Shell(ShellConfig::new("cargo test"))),
                ("lint", StepConfig::Shell(ShellConfig::new("cargo clippy"))),
                ("ping", StepConfig::Http(HttpConfig::get("https://example.com/health"))),
            ],
            true,
        )
        .await?;
    for r in &results {
        let _ = (r.name.as_str(), r.output.is_success());
    }
    Ok(())
}
```

## Secrets

Encrypted at rest, namespaced per workflow, created in the dashboard under Secrets.
Requires `IRONFLOW_SECRET_KEYS` on the server and the `secret-store` feature on the
engine in the workflows crate:

```bash
cargo add -p workflows ironflow-engine --features secret-store
```

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let token = ctx
        .secrets()
        .get("gitlab_token")
        .await
        .map_err(EngineError::Store)?
        .ok_or_else(|| EngineError::StepConfig("secret gitlab_token missing".to_string()))?;
    // Through the environment only. Never in the command string, never echoed.
    ctx.shell(
        "push",
        ShellConfig::new("glab auth status").env("GITLAB_TOKEN", &token.value),
    )
    .await?;
    Ok(())
}
```

## Artifacts

Files a step produces are collected by glob and handed to later steps.

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    ctx.shell(
        "report",
        ShellConfig::new("./gen-report > report.html").dir("/app").output("report.html"),
    )
    .await?;
    // The file is written into the working directory before the command runs.
    ctx.shell(
        "publish",
        ShellConfig::new("./publish report.html").dir("/app").input("report", "report.html"),
    )
    .await?;
    Ok(())
}
```

A declared output that matches no file fails the step. Artifacts need `ARTIFACTS_DIR` on
the server.

## Error handler

Fires once when any later step fails; the original error is preserved.

```rust,no_run
use ironflow_engine::config::{ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    ctx.on_error(
        "notify-failure",
        StepConfig::Shell(ShellConfig::new("./notify.sh failed")),
    );
    ctx.shell("risky", ShellConfig::new("./migrate.sh")).await?;
    Ok(())
}
```

## Handler metadata

| Method | Default | Use |
|---|---|---|
| `description()` | `""` | Shown in dashboard and CLI |
| `source_code()` | `None` | `Some(include_str!("file.rs"))` |
| `category()` | `None` | `"data/etl"` groups workflows in the UI tree |
| `input_schema()` | `None` | `Some(input_schema_for::<T>())` |
| `default_labels()` | empty | Labels applied to every run |
| `schedule()` | `None` | `CronSchedule`, wired by the runtime |
| `default_max_cost_usd()` | `None` | Cost cap for runs of this handler |
| `version()` / `compatible_versions()` | `"1"` / empty | Retry compatibility across handler versions |
| `sub_workflows()` | empty | Names of handlers invoked through `ctx.workflow` |
| `guard_config()` | `None` | Recursion depth, fan-out, token and time guards |

`describe()` assembles all of them. Override it only for metadata the methods cannot express.
