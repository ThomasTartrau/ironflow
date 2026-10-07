# Standalone Runtime

`ironflow-runtime` is the path without a database: a small axum server that exposes webhook
endpoints and calls `ironflow-core` operations directly. There is no store, no worker and no
dashboard, so nothing is persisted. Use it for a webhook that triggers a quick job; use the
full platform when you need run history, approvals or retries.

```rust,no_run
use ironflow_core::prelude::*;
use ironflow_runtime::prelude::*;

async fn on_push(payload: serde_json::Value, provider: &ClaudeCodeProvider) {
    let branch = payload["ref"].as_str().unwrap_or("main");
    // The branch comes from the webhook: an argument, never a `sh -c` string.
    let range = format!("origin/main...origin/{branch}");
    let diff = Shell::exec("git", &["diff", &range])
        .await
        .expect("git diff");
    let review = Agent::new()
        .prompt(&format!("Review this diff:\n{}", diff.stdout()))
        .model(Model::SONNET)
        .max_budget_usd(0.50)
        .run(provider)
        .await
        .expect("agent review");
    println!("{}", review.text());
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ClaudeCodeProvider::new();

    Runtime::new()
        .webhook("/hooks/github", WebhookAuth::github("my-secret"), {
            let p = provider.clone();
            move |payload| {
                let p = p.clone();
                async move { on_push(payload, &p).await }
            }
        })
        .serve("0.0.0.0:8080")
        .await?;

    Ok(())
}
```

| Webhook auth | Behaviour |
|--------------|-----------|
| `WebhookAuth::none()` | No authentication |
| `WebhookAuth::header(name, value)` | Static header comparison |
| `WebhookAuth::github(secret)` | GitHub HMAC-SHA256 (`X-Hub-Signature-256`) |
| `WebhookAuth::gitlab(secret)` | GitLab token (`X-Gitlab-Token`) |

Built-in endpoints: `GET /health`, and `GET /metrics` with the `prometheus` feature.

## Webhook replays

GitHub and GitLab send a delivery again when the receiver times out or answers 5xx. Use
`webhook_with_context` to get the provider delivery id, already prefixed and ready to pass as
an [`Idempotency-Key`](../concepts/runs.md#idempotent-runs), so a replay does not start a
second run.

```rust,no_run
use ironflow_runtime::prelude::*;

Runtime::new().webhook_with_context(
    "/hooks/github",
    WebhookAuth::github("my-secret"),
    |ctx: WebhookContext| async move {
        // ctx.delivery_id == Some("github:8f4e2a10-...") when the provider
        // stamped the request, None otherwise. Pass it straight to
        // create_run_idempotent as the Idempotency-Key.
        match ctx.delivery_id {
            Some(key) => println!("replay-safe key: {key}"),
            None => println!("no delivery id, the call is not replay-safe"),
        }
    },
);
```

| Provider | Header read | Derived key |
|----------|-------------|-------------|
| GitHub | `X-GitHub-Delivery` | `github:<id>` |
| GitLab | `X-Gitlab-Event-UUID` | `gitlab:<id>` |

`webhook()` keeps its original signature and receives only the payload.

## Metrics

With the `prometheus` feature, operations record these metrics, in the runtime as in a
worker:

| Metric | Type | Labels |
|--------|------|--------|
| `ironflow_shell_total` | Counter | `status` |
| `ironflow_shell_duration_seconds` | Histogram | |
| `ironflow_http_total` | Counter | `method`, `status` |
| `ironflow_http_duration_seconds` | Histogram | |
| `ironflow_agent_total` | Counter | `model`, `status` |
| `ironflow_agent_duration_seconds` | Histogram | `model` |
| `ironflow_agent_cost_usd_total` | Gauge | `model` |
| `ironflow_agent_tokens_input_total` | Counter | `model` |
| `ironflow_agent_tokens_output_total` | Counter | `model` |
| `ironflow_agent_tokens_cache_read_total` | Counter | `model` |
| `ironflow_agent_tokens_cache_write_total` | Counter | `model` |
| `ironflow_webhook_received_total` | Counter | `path`, `auth` |
| `ironflow_runs_reaped_total` | Counter | `outcome` |
| `ironflow_worker_leases_lost_total` | Counter | |
