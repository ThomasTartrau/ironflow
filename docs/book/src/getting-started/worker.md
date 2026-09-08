# Running a Worker

Workers poll the API for pending runs, acquire leases, and execute workflow handlers.

## Example worker

```rust,ignore
{{#include ../../../../examples/worker/src/main.rs}}
```

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `API_URL` | `http://localhost:3000` | Address of the API server |
| `WORKER_TOKEN` | dev token | Shared secret matching the server |
| `CONCURRENCY` | `2` | Number of parallel runs |
| `POLL_INTERVAL_SECS` | `2` | Seconds between polls |

## Running

```sh
cargo run -p ironflow-example-worker
```

## Scaling

To increase throughput, start multiple workers. Each worker polls independently and acquires leases on runs, so no coordination is needed beyond the API server.
