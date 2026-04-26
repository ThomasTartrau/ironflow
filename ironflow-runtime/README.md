# ironflow-runtime

Daemon/server layer for **ironflow**, providing webhook HTTP endpoints and cron scheduling on top of `ironflow-core` operations.

## Features

- **Webhook routes** with pluggable authentication (GitHub, GitLab, HMAC-SHA256, static header)
- **Cron scheduling** via `tokio-cron-scheduler`
- **Graceful shutdown** with signal handling
- **Prometheus metrics** (optional `prometheus` feature)

## Quick start

```rust,no_run
use ironflow_runtime::prelude::*;

async fn example() -> Result<(), RuntimeError> {
    Runtime::builder()
        .port(3000)
        .webhook("/github", WebhookAuth::github("secret"), |payload| async move {
            // handle webhook
            Ok(())
        })
        .cron("cleanup", "0 0 * * *", || async {
            // daily cleanup
            Ok(())
        })
        .build()
        .serve()
        .await
}
```

## Feature flags

| Feature | Description |
|---------|-------------|
| `prometheus` | Expose webhook and cron metrics via Prometheus |

## License

MIT License - see [LICENSE](../LICENSE) for details.
