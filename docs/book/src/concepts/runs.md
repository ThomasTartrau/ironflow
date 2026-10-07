# Runs

A Run is one execution of a workflow. It is created by a trigger, waits in the queue as
`pending`, is picked by a worker, and ends `completed`, `failed`, `warning` or `cancelled`.
Every step it executes is persisted with its output, duration and cost.

This page covers how runs are created and the options that control it. How workers pick
them, with [concurrency groups](engine-worker.md#concurrency-groups) and
[priority](engine-worker.md#run-priority), is in [Engine & Worker](engine-worker.md).

## Triggers

| Trigger | Source |
|---------|--------|
| `Manual` | CLI or a direct programmatic call |
| `Api` | `POST /api/v1/runs` |
| `Webhook { path }` | Incoming webhook, authenticated per route |
| `Cron { schedule }` | Cron expression, see [Schedules](schedules.md) |
| `Retry { parent_run_id }` | Retry of a previously failed run |
| `Workflow` | Invoked as a sub-workflow step by a parent run |

A run can also carry a `scheduled_at`: set it at creation and the run stays pending until
that time.

## Idempotent runs

`POST /api/v1/runs` accepts an optional `Idempotency-Key` header. Sending the same key again
returns the run it already created instead of starting a second one. It protects against
webhook replays and client retries after a network timeout.

| Situation | Response |
|-----------|----------|
| No header | `201 Created`, a new run every time |
| Key unknown | `201 Created` with the new run |
| Key known, same workflow and payload | `200 OK` with the original run |
| Key known, different workflow or payload | `409 IDEMPOTENCY_KEY_CONFLICT` |

Rules:

- A key is at most **255 printable ASCII characters** and must not be empty.
- A key is **global**, not scoped per workflow: reusing one across two workflows
  is a conflict. Prefix it (`github:...`) to keep sources apart.
- A key stays bound for **24 hours**. Past that window it is released and the
  next call with it creates a new run.
- The run returned on replay is the original one **whatever its state**, including
  `failed` or `cancelled`. Use `POST /api/v1/runs/:id/retry` to run it again
  deliberately; a retry never inherits the key.
- Only the workflow and the payload decide replay versus conflict. Labels are
  merged with the handler defaults server-side and are not compared.

```bash
# Same key twice: one run, the second call answers 200.
curl -X POST https://ironflow.example.com/api/v1/runs \
  -H "Authorization: Bearer $TOKEN" \
  -H "Idempotency-Key: github:8f4e2a10" \
  -H "Content-Type: application/json" \
  -d '{"workflow": "deploy", "payload": {"env": "prod"}}'
```

A conflict names the run holding the key:

```json
{
  "error": {
    "code": "IDEMPOTENCY_KEY_CONFLICT",
    "message": "idempotency key already used with a different request",
    "details": { "run_id": "0199c3f0-..." }
  }
}
```

The SDK, the CLI and the MCP server all carry the key:

```rust,no_run
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::CreateRunRequest;

# async fn example(request: &CreateRunRequest) -> Result<(), Box<dyn std::error::Error>> {
let client = IronflowClient::new("http://localhost:3000", "irfl_...");
client.create_run_idempotent(request, "github:8f4e2a10").await?;
# Ok(())
# }
```

```bash
ironflow-cli run create deploy --payload '{"env":"prod"}' \
  --idempotency-key github:8f4e2a10
```

A webhook received by the [standalone runtime](../guides/standalone-runtime.md#webhook-replays)
exposes the provider delivery id, ready to use as the key.

With the `prometheus` feature, `ironflow_run_idempotency_total{outcome}` counts
`created`, `replayed` and `conflict` outcomes.

## Exclusive runs

`POST /api/v1/runs` also accepts an optional `concurrency_key` in the body. While a run
holding the same key is not finished (pending, running, sleeping, retrying, awaiting
approval), a second creation is refused with `409 CONCURRENCY_CONFLICT` naming that run. The
key is released when the holder completes, fails, ends with a warning or is cancelled. Use it
to keep one active run per issue, branch or tenant.

```bash
curl -X POST https://ironflow.example.com/api/v1/runs \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"workflow": "fix-issue", "payload": {"issue": 12}, "concurrency_key": "issue:12"}'
```

```json
{
  "error": {
    "code": "CONCURRENCY_CONFLICT",
    "message": "concurrency key \"issue:12\" is held by active run 0199c3f0-...",
    "details": { "key": "issue:12", "run_id": "0199c3f0-..." }
  }
}
```

- A key is at most **255 bytes** and must not be empty. It is global, like an
  idempotency key: prefix it to keep sources apart.
- An idempotent replay (same `Idempotency-Key`, same request) is answered before the
  exclusivity check, so it returns the original run instead of a conflict.
- A retry (`POST /api/v1/runs/:id/retry`) inherits the key and answers `409` if an
  active run took it since.
- The CLI takes `--concurrency-key issue:12` and the MCP `create_run` tool a
  `concurrency_key` argument.

A sub-workflow takes the key through `ctx.workflow_with`; a conflict completes the step with
the run holding the key instead of failing the parent (see
[Exclusive children](steps.md#exclusive-children)).

A concurrency key **refuses** a second run. To **queue** it instead, use a
[concurrency group](engine-worker.md#concurrency-groups).
