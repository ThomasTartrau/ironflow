# Steps

A Step is an atomic unit of work within a Run. Each step is persisted in the database with its input, output, status, cost, duration, and token counts.

## Step kinds

| Kind | Method | Description |
|------|--------|-------------|
| Shell | `ctx.shell()` | Execute a shell command |
| Http | `ctx.http()` | Make an HTTP request |
| Agent | `ctx.agent()` | Call an AI agent (Claude, OpenAI, etc.) |
| Approval | `ctx.approval()` | Pause for human approval |
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
