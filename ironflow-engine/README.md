# ironflow-engine

Workflow orchestration engine for **ironflow** with FSM-based run lifecycle management.

## Overview

Workflows are defined as Rust-native handlers implementing `WorkflowHandler`. Handlers receive a `WorkflowContext` and can:

- Chain step outputs via plain variables
- Use native `if`/`else`/`match` for conditional branching
- Execute steps in parallel
- Require human approval before critical steps

## Key components

| Component | Description |
|-----------|-------------|
| `Engine` | Registers workflows and dispatches runs |
| `WorkflowContext` | Per-run context providing step execution, approval, and logging |
| `WorkflowHandler` | Trait to implement for custom workflows |
| `Operation` | Trait for custom step types beyond built-in Shell/Agent/Http |
| FSM (`fsm/`) | Finite state machine governing run lifecycle transitions |

## Run lifecycle

```
Created -> Queued -> Running -> Completed
                  \-> WaitingApproval -> Running
                  \-> Failed
                  \-> Cancelled
```

## Feature flags

| Feature | Description |
|---------|-------------|
| `prometheus` | Expose engine and execution metrics |
| `openapi` | Derive `utoipa::ToSchema` on public types |

## Quick start

```rust,no_run
use ironflow_engine::prelude::*;

struct MyWorkflow;

impl WorkflowHandler for MyWorkflow {
    async fn handle(&self, ctx: WorkflowContext) -> Result<(), WorkflowError> {
        let output = ctx.shell("echo hello").await?;
        ctx.log(&output.stdout).await;
        Ok(())
    }
}
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
