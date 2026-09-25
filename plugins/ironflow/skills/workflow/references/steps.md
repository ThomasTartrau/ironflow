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
use ironflow_engine::config::{AgentStepConfig, Tool};
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
    // Structured output: the provider is constrained to the schema of `Review`,
    // and the step returns the `Review` itself.
    let review = ctx
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
    let _ = (review.score, review.summary);

    // Tools: the agent can act, and answers in free text. `Tool::Custom` carries
    // an MCP tool or a permission pattern.
    let explore = ctx
        .agent(
            "explore",
            AgentStepConfig::new("List the top-level files and summarise the project.")
                .allow_tool(Tool::Bash)
                .allow_tool(Tool::Read)
                .max_turns(6)
                .max_budget_usd(0.50)
                .verbose(true),
        )
        .await?;
    let _text = explore.text();
    Ok(())
}
```

A structured answer that does not match its type fails the step with
`EngineError::Serialization`.

`Model::SONNET`, `Model::OPUS`, `Model::HAIKU` are aliases resolved by the provider;
pass a full model id string for a pinned version. `verbose(true)` records the tool
timeline shown in the dashboard.

## Approval

```rust,no_run
use std::time::Duration;

use ironflow_engine::config::{ApprovalConfig, Assignee, EscalationPolicy};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    ctx.approval(
        "approve-production",
        ApprovalConfig::new("Staging looks good. Deploy to production?")
            .assigned_to(Assignee::group("release-managers"))
            .with_deadline(Duration::from_secs(3600))
            .on_timeout(EscalationPolicy::AutoReject),
    )
    .await?;
    Ok(())
}
```

The run moves to `AwaitingApproval`. `POST /api/v1/runs/{id}/approve` (dashboard, CLI,
MCP) resumes it: the handler is replayed from the top. Rejection fails the run. Read
`approval-replay.md` before putting code between steps around a gate.

### SLA and escalation

`with_deadline` (or `with_deadline_secs`) arms a timer persisted on the step, so it
survives an API or worker restart. `on_timeout` says what happens when it fires;
without one, an expired deadline auto-rejects. `assigned_to` takes an `Assignee`
(`Assignee::user("alice")` or `Assignee::group("sre-oncall")`) recording who is
expected to answer; it shows up in the API, the dashboard and `ironflow run steps`.
The assignee is advisory (notification/audit), not an authorization check.

| `EscalationPolicy` | On expiry |
|--------------------|-----------|
| `AutoApprove` | Completes the gate as `system:timeout` and resumes the run |
| `AutoReject` | Fails the step and the run with `approval timeout` (default) |
| `Notify(targets)` | Posts the event to each `NotificationTarget`, keeps the gate open, restarts the timer |
| `Escalate(Assignee)` | Reassigns the gate to a user or group, keeps it open, restarts the timer |
| `Chain(policies)` | One policy per expiry, in order |

On their own, `Notify` and `Escalate` repeat at every expiry until a human answers;
inside a `Chain` they advance to the next policy instead. Once a chain runs out, the
gate stays open with no timer — it is never silently auto-rejected.

```rust,no_run
use std::time::Duration;

use ironflow_engine::config::{ApprovalConfig, EscalationPolicy, NotificationTarget};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn escalating(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    ctx.approval(
        "approve-production",
        ApprovalConfig::new("Deploy to production?")
            .with_deadline(Duration::from_secs(3600))
            .on_timeout(EscalationPolicy::Chain(vec![
                // After 1 h: ping the on-call channel, keep waiting.
                EscalationPolicy::Notify(vec![NotificationTarget::Slack {
                    webhook_url: "https://hooks.slack.com/services/T/B/X".to_string(),
                    channel: "#deploys".to_string(),
                }]),
                // After 2 h: give up.
                EscalationPolicy::AutoReject,
            ])),
    )
    .await?;
    Ok(())
}
```

Every firing is recorded in the audit log as an `approval_escalated` event with the
stage, the policy, what it did and why.

### Several approvers

`requiring` makes the number of approvals, and who may give them, depend on the run.
Compute the `Approvers` in plain Rust from the typed input and earlier step outputs;
the gate records them when it opens and keeps them on replay.

```rust,no_run
use ironflow_engine::config::{ApprovalConfig, Approvers};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use serde::Deserialize;

#[derive(Deserialize)]
struct Payment {
    amount: u64,
}

async fn payment_gate(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let payment: Payment = ctx.input().await?;
    let approvers = match payment.amount {
        // Three approvers from finance or the board above 100k.
        a if a > 100_000 => Approvers::at_least(3)
            .from_groups(["finance", "board"])
            .because("amount > 100k"),
        // Two distinct approvers from finance above 10k.
        a if a > 10_000 => Approvers::at_least(2)
            .from_groups(["finance"])
            .because("amount > 10k"),
        _ => Approvers::any(),
    };
    ctx.approval(
        "release-payment",
        ApprovalConfig::new("Release the payment?").requiring(approvers),
    )
    .await?;
    Ok(())
}
```

`because(..)` is an audit label shown on the dashboard, never evaluated.
`Approvers::at_least(0)` and a blank group name panic.

Each user votes once (an admin's vote counts as one); a single rejection fails the run.
Group membership is managed by admins with `ironflow user set-groups <id> --group
finance`.

## Decision

A typed machine decision (System One / Jev): classify, route, score, or yes/no, with a
calibrated confidence instead of free text. Cheaper and faster than an agent for a
structured verdict. Wire a `DecisionProvider` into the worker that runs the workflow
(`WorkerBuilder::decision_provider(...)`), or into the engine
(`Engine::with_decision_provider(...)`); without one, a decision step fails with
`NoDecisionProvider`.

The questions are the fields of a struct deriving `DecisionAnswers`; the options of a
choice are the unit variants of an enum deriving `DecisionChoice` (label: the variant in
`snake_case`, or `#[choice(rename = "..")]`; `#[choice(description = "..")]` is sent to
the model). `ctx.decision` returns the struct.

```rust,no_run
use ironflow_engine::config::{DecisionConfig, ShellConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::decision::{DecisionAnswers, DecisionChoice};
use ironflow_engine::error::EngineError;

#[derive(Debug, DecisionChoice)]
enum Team {
    #[choice(description = "Payments, invoices, refunds")]
    Billing,
    Technical,
    Sales,
}

#[derive(Debug, DecisionAnswers)]
struct Triage {
    /// f64 in [0, 1]: probability of "yes".
    #[noul("Does this convey urgency?")]
    is_urgent: f64,
    #[choice("Which team?")]
    team: Team,
    /// f64: probability-weighted level index.
    #[score("How frustrated?", levels = ["Calm", "Frustrated", "Very angry"])]
    mood: f64,
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let triage = ctx
        .decision(
            "triage",
            DecisionConfig::new("Payouts have been failing for 3 days")
                .answers::<Triage>()
                // Below 0.7 confidence, suspend for human review.
                .escalate_below(0.7),
        )
        .await?;

    if triage.is_urgent > 0.8 && triage.mood > 1.5 {
        ctx.shell(
            "route",
            ShellConfig::new("echo \"routed to $TEAM\"").env("TEAM", triage.team.label()),
        )
        .await?;
    }
    Ok(())
}
```

A field without a question, a score without levels, or a field whose type does not fit
its question does not compile. An option the provider returns that is not a variant fails
the step.

When any answer's confidence falls below `escalate_below`, the run moves to
`AwaitingApproval` exactly like an approval gate. On resume the decision is **not**
re-run: the stored answers are replayed as-is, so the routing above stays stable.

## Sub-workflow

A child called as a sub-workflow declares its input type with `TypedWorkflow`; the parent
can then only pass that type.

```rust,no_run
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::executor::StepOutput;
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

struct Collect;

impl WorkflowHandler for Collect {
    fn name(&self) -> &str {
        "collect"
    }
    fn input_schema(&self) -> Option<Value> {
        Self::typed_input_schema()
    }
    fn execute<'a>(&'a self, _ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move { Ok(()) })
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
            // Read the child's steps through the typed accessors.
            for step in ctx.store().list_steps(child.run_id()).await? {
                let _stdout = StepOutput::from(&step).stdout().to_string();
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

`child.run_id()` is a `Uuid` (nil while planning). A child never sees the parent's
artifacts; pass what it needs in its input.

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

## Conditions

Branching is plain Rust `if`/`else`. `ctx.when` and `ctx.when_dynamic` make a
branch visible to `ironflow run plan` without changing what the handler does.

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use serde::Deserialize;

#[derive(Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum Env {
    Prod,
    Staging,
}

#[derive(Deserialize)]
struct DeployInput {
    env: Env,
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    // Resolved against the typed run input: the plan reports it as `evaluated`,
    // labelled "production run". A payload that is not a `DeployInput` fails.
    if ctx
        .when("production run", |i: &DeployInput| i.env == Env::Prod)
        .await?
    {
        ctx.shell("deploy-prod", ShellConfig::new("./deploy prod")).await?;
    } else {
        ctx.skip("deploy-prod", "not a production run").await?;
    }

    // Depends on a step output: the plan reports it as `unevaluable`.
    let build = ctx.shell("build", ShellConfig::new("cargo build")).await?;
    if ctx.when_dynamic("build succeeded", build.is_success()) {
        ctx.shell("notify", ShellConfig::new("./notify ok")).await?;
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

Files a step produces are collected by glob and handed to later steps through a
handle the producing step gives out.

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;

async fn example(ctx: &mut WorkflowContext, summarize: &dyn Operation) -> Result<(), EngineError> {
    let report = ctx
        .shell(
            "report",
            ShellConfig::new("./gen-report > out/report.html").dir("/app").output("out/*.html"),
        )
        .await?;
    // The handle is named after the file. A name the step did not declare fails here.
    let html = report.artifact("report.html")?;

    // The file is written into the working directory before the command runs.
    ctx.shell(
        "publish",
        ShellConfig::new("./publish report.html").dir("/app").input(&html),
    )
    .await?;

    // Custom operations store bytes by hand and get a handle back.
    let summary = ctx.operation("summarize", summarize).await?;
    let json = ctx
        .put_artifact(&summary, "summary.json", None, br#"{"ok":true}"#.to_vec())
        .await?;
    let _bytes = ctx.get_artifact(&json).await?;
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
| `input_schema()` | `None` | `Some(input_schema_for::<T>())`, or `Self::typed_input_schema()` with `TypedWorkflow` |
| `default_labels()` | empty | Labels applied to every run |
| `schedule()` | `None` | `CronSchedule`, wired by the runtime |
| `default_max_cost_usd()` | `None` | Cost cap for runs of this handler |
| `version()` / `compatible_versions()` | `"1"` / empty | Retry compatibility across handler versions |
| `sub_workflows()` | empty | `sub_workflow_names(&[&Child])`: the handlers invoked through `ctx.workflow` |
| `guard_config()` | `None` | Recursion depth, fan-out, token and time guards |

`describe()` assembles all of them. Override it only for metadata the methods cannot express.
