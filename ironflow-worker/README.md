# ironflow-worker

HTTP-based background worker for **ironflow**. Polls the API server for pending workflow runs, executes them locally, and reports results back.

## How it works

1. The worker polls `GET /api/v1/internal/runs/next` for queued runs
2. Claims a run via `POST /api/v1/internal/runs/:id/claim`
3. Executes the workflow using the registered `WorkflowHandler`
4. Pushes step logs in real-time via `POST /api/v1/internal/runs/:id/logs`
5. Reports final status (completed/failed) back to the API

Communication is authenticated with a static `WORKER_TOKEN`.

## Feature flags

| Feature | Description |
|---------|-------------|
| `prometheus` | Expose worker execution metrics |
| `heartbeat` | Periodic heartbeat to the API server |

## Quick start

```rust,no_run
use std::sync::Arc;
use std::time::Duration;
use ironflow_worker::WorkerBuilder;
use ironflow_core::providers::claude::ClaudeCodeProvider;

async fn example() {
    let provider = Arc::new(ClaudeCodeProvider::default());
    let worker = WorkerBuilder::new("http://localhost:3000", "worker-token")
        .provider(provider)
        .poll_interval(Duration::from_secs(5))
        .build();

    worker.run().await.unwrap();
}
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
