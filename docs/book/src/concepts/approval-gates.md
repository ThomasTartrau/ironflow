# Approval Gates

An approval gate pauses a workflow run until a human approves or rejects it. This enables human-in-the-loop workflows like deploy pipelines where production deploys require sign-off.

## How it works

1. The handler calls `ctx.approval()` with a prompt message
2. The run transitions to `AwaitingApproval`
3. The worker releases the run and moves on to other work
4. A human calls `POST /api/v1/runs/:id/approve` or `POST /api/v1/runs/:id/reject`
5. On approval, the run is requeued. A worker picks it up, replays completed steps from cache, skips the approved gate, and continues execution
6. On rejection, the run transitions to `Failed`

## Example

```rust,ignore
{{#include ../../../../examples/ironflow-workflows/src/deploy_approval.rs}}
```

## Configuration

```rust,ignore
ApprovalConfig::new("Deploy to production?")
    .with_timeout_seconds(3600)  // Auto-reject after 1 hour
```

The timeout is optional. Without it, the run waits indefinitely.

## Step replay

After an approval, the engine re-executes the handler from the beginning. Completed steps return their cached output immediately -- they do not re-run. The approved gate is skipped, and execution resumes with the next step.
