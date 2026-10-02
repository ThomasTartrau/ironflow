# Signals

A **signal** is an external message sent to Ironflow to resume the runs waiting
for it. It has a **name** (what happened, e.g. `ci.pipeline_finished`) and a
**key** (which occurrence, e.g. a commit SHA). A run waits with
`ctx.wait_for_signal`; a producer delivers with `POST /api/v1/signals`, the CLI,
the MCP server or `Engine::send_signal`.

## Declaring a signal

A signal is a typed payload: a struct implementing `Signal`, which names it.

```rust,ignore
use ironflow_engine::signal::Signal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, JsonSchema)]
struct PipelineFinished {
    status: String,
}

impl Signal for PipelineFinished {
    const NAME: &'static str = "ci.pipeline_finished";
}
```

## Waiting for a signal

```rust,ignore
let finished = ctx
    .wait_for_signal::<PipelineFinished>("wait-ci", &sha, Duration::from_secs(3600))
    .await?;
match finished {
    Some(pipeline) if pipeline.status == "success" => { /* deploy */ }
    Some(_) => return Err(EngineError::StepConfig("CI failed".to_string())),
    None => return Err(EngineError::StepConfig("CI timed out".to_string())),
}
```

- **The key is the occurrence**: wait on a commit SHA, not on a merge request.
  A signal for an older push must never resume a run waiting for the newest one.
- **Received early**: a signal delivered after the run was created but before the
  step opened resolves the step at once, without suspending.
- **Suspended**: otherwise the run goes `Sleeping` until the deadline. A delivery
  whose payload matches the JSON schema of the type wakes it, and the handler
  receives `Some(payload)`.
- **Timeout**: when the deadline passes first, the step completes as timed out and
  the handler receives `None`.
- **Broadcast**: every run waiting on the same `(name, key)` receives the signal.
- **Invalid payload**: a run whose schema the payload does not match keeps waiting
  and is listed under `rejected` in the response. The signal is still stored.
- **Idempotency**: a second delivery with the same `idempotency_id` returns
  `duplicate: true` and delivers nothing.

Woken runs resume in-process under `ExecutionMode::Local`, or go back to
`Pending` for a worker under `ExecutionMode::Workers`. Timeouts are applied by the
waker task of the API server (see [Engine and worker](engine-worker.md)).

## Sending a signal

| Channel | How |
|---------|-----|
| REST | `POST /api/v1/signals` with `{"name", "key", "payload", "idempotency_id"}`; admin JWT or API key with the `signals_send` scope |
| REST | `GET /api/v1/signals?name=&key=` lists received signals (`runs_read` for an API key) |
| CLI | `ironflow signal send ci.pipeline_finished --key <sha> --payload '{"status":"success"}'` |
| CLI | `ironflow signal list --name ci.pipeline_finished` |
| MCP | `send_signal`, `list_signals` |
| Rust | `engine.send_signal(&PipelineFinished { .. }, &sha, Some(&delivery_id))` |

## Example: wait for CI

The workflow pushes a commit and waits for its pipeline:

```rust,ignore
ctx.shell("push", ShellConfig::new("git push origin HEAD")).await?;
let finished = ctx
    .wait_for_signal::<PipelineFinished>("wait-ci", &sha, Duration::from_secs(3600))
    .await?;
```

The CI webhook handler filters the terminal pipeline statuses and forwards them,
using the delivery ID of the webhook so a retried delivery is not counted twice:

```rust,ignore
if matches!(status.as_str(), "success" | "failed" | "canceled") {
    engine
        .send_signal(&PipelineFinished { status }, &sha, Some(&delivery_id))
        .await?;
}
```

Signals are kept `SIGNAL_RETENTION_DAYS` days (default 7) and then purged. The example server wires the variable into its `RunPurger`; a custom server builds it with `RunPurger::from_config(store, &config)`.
