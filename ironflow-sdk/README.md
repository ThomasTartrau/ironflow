# ironflow-sdk

Type-safe Rust SDK for the **ironflow** workflow engine API. Generated from the OpenAPI specification via [progenitor](https://github.com/oxidecomputer/progenitor), with ergonomic wrappers for authentication and SSE event streaming.

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `rustls` | yes | TLS via rustls (pure Rust) |
| `native-tls` | no | TLS via the platform's native stack |

## Quick start

```rust,no_run
use ironflow_sdk::IronflowClient;

async fn example() -> Result<(), ironflow_sdk::Error> {
    let client = IronflowClient::new("https://ironflow.example.com", "my-api-key");

    // List runs
    let runs = client.list_runs().await?;
    for run in &runs.data {
        println!("{}: {}", run.workflow_name, run.status);
    }

    // Get stats
    let stats = client.get_stats().await?;
    println!("Total runs: {}", stats.data.total_runs);

    Ok(())
}
```

## Available methods

| Category | Methods |
|----------|---------|
| Runs | `list_runs`, `list_runs_filtered`, `create_run`, `get_run`, `cancel_run`, `approve_run`, `reject_run`, `retry_run` |
| Workflows | `list_workflows`, `get_workflow` |
| Stats | `get_stats` |
| Auth | `sign_in`, `sign_out`, `me` |
| API Keys | `list_api_keys`, `create_api_key`, `available_scopes`, `delete_api_key` |
| Users | `list_users`, `create_user`, `delete_user`, `update_role` |
| Secrets | `list_secrets`, `create_secret`, `update_secret`, `delete_secret` |
| Audit | `list_audit_logs` |
| SSE | `events` (real-time event streaming) |
| Health | `health_check` |

## SSE event streaming

```rust,no_run
use ironflow_sdk::IronflowClient;
use futures_util::StreamExt;

async fn example() -> Result<(), ironflow_sdk::Error> {
    let client = IronflowClient::new("https://ironflow.example.com", "my-api-key");
    let mut stream = client.events(None, None).await?;

    while let Some(event) = stream.next().await {
        let event = event?;
        println!("[{}] {}", event.event_type, event.data);
    }

    Ok(())
}
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
