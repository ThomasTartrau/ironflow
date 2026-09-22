# Steps

A Step is an atomic unit of work within a Run. Each step is persisted in the database with its input, output, status, cost, duration, and token counts.

## Step kinds

| Kind | Method | Description |
|------|--------|-------------|
| Shell | `ctx.shell()` | Execute a shell command |
| Http | `ctx.http()` | Make an HTTP request |
| Agent | `ctx.agent()` | Call an AI agent (Claude, OpenAI, etc.) |
| Approval | `ctx.approval()` | Pause for human approval |
| Decision | `ctx.decision()` | Make a typed machine decision ([System One / Jev](decision.md)) |
| Workflow | `ctx.workflow()` | Start a sub-workflow |
| Custom | `ctx.operation()` | Run a custom [Operation](operations.md) |

## Shell steps

```rust,ignore
let output = ctx.shell("build", ShellConfig::new("cargo build")).await?;
if output.is_success() {
    // continue
}
```

## HTTP steps

```rust,ignore
let response = ctx.http("fetch-data", HttpConfig::get("https://api.example.com/data")).await?;
```

## Agent steps

```rust,ignore
let result = ctx.agent("analyze", AgentStepConfig::new("Analyze this log file")).await?;
```

## Approval steps

```rust,ignore
ctx.approval("prod-gate", ApprovalConfig::new("Deploy to production?")).await?;
```

The run suspends on `AwaitingApproval` until a human answers. A gate can also
carry an SLA: a deadline persisted on the step and an escalation policy applied
when it expires.

```rust,ignore
use std::time::Duration;

ctx.approval(
    "prod-gate",
    ApprovalConfig::new("Deploy to production?")
        .assigned_to(Assignee::group("release-managers"))
        .with_deadline(Duration::from_secs(3600))
        .on_timeout(EscalationPolicy::AutoReject),
).await?;
```

| Field | Builder | Meaning |
|-------|---------|---------|
| `message` | `ApprovalConfig::new` | Prompt shown to reviewers |
| `assignee` | `assigned_to` | `Assignee::user` / `Assignee::group` expected to answer |
| `deadline_secs` | `with_deadline` / `with_deadline_secs` | SLA window, in seconds |
| `on_timeout` | `on_timeout` | `EscalationPolicy` applied when the deadline fires (defaults to `AutoReject`) |
| `timeout_seconds` | `with_timeout_seconds` | Legacy spelling of a deadline with an implicit `AutoReject` |

See [Approval Gates](approval-gates.md) for the full list of escalation policies
and where the remaining time surfaces.

## Decision steps

A decision step asks a [`DecisionProvider`](decision.md) (System One / Jev) a map of
typed questions about a state and returns typed answers with a calibrated confidence.

```rust,ignore
let out = ctx.decision(
    "triage",
    DecisionConfig::new("Payouts have been failing for 3 days")
        .choice("team", "Which team?", &["billing", "technical"])
        .escalate_below(0.7),
).await?;
let team = &out.choice("team")?.choice;
```

Below the `escalate_below` confidence threshold, the run suspends for human approval;
on resume the stored answers are replayed as-is. See [Decisions](decision.md).

## Conditions

A handler branches with plain Rust `if`/`else`. That is invisible to the
[execution planner](../guides/execution-plan.md), which is why two helpers
exist to declare a branch explicitly.

`ctx.when(expression, predicate)` evaluates a predicate against the run input.
It returns the predicate's value, and the planner records it as `evaluated`
together with the expression you named:

```rust,ignore
if ctx.when("input.env == 'prod'", |p| p["env"] == "prod").await? {
    ctx.shell("deploy-prod", ShellConfig::new("./deploy prod")).await?;
} else {
    ctx.skip("deploy-prod", "not a production run").await?;
}
```

`ctx.when_dynamic(expression, value)` declares a branch whose value comes from
a previous step's output. It returns `value` unchanged; the planner records the
condition as `unevaluable`, because step outputs are synthetic while planning:

```rust,ignore
let build = ctx.shell("build", ShellConfig::new("cargo build")).await?;
if ctx.when_dynamic("build succeeded", build.is_success()) {
    ctx.shell("deploy", ShellConfig::new("./deploy")).await?;
}
```

Both helpers are optional: a plain `if` still runs exactly the same way. They
only make the branch legible to whoever reads the plan.

## Step status lifecycle

Steps follow this state machine:

```mermaid
stateDiagram-v2
    [*] --> Pending
    Pending --> Running
    Running --> Completed
    Running --> Failed
    Pending --> Skipped
```

Every step transition is recorded. Failed steps report their error in the step output.
