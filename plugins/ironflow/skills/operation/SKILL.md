---
name: operation
description: Write a custom Ironflow Operation - an integration (GitLab, Slack, any HTTP API) executed as a tracked step through ctx.operation(). Loaded by the ironflow hub for the operation verb.
user-invocable: false
---

# Ironflow operation

`Operation` is the extension point for step kinds Ironflow does not ship. The engine owns the lifecycle (step record, status, duration, output persistence); the operation owns the call.

Built-in kinds already exist for a plain shell command, an HTTP request and an agent. Write an operation when the step needs its own struct, its own error mapping, or its own structured output. A single `ctx.http()` is enough for a one-off request.

## 1. Gather the shape

One AskUserQuestion call: operation name (`snake_case` struct, lowercase `kind()`), the external call it makes, its inputs, what the step output must contain, and where the credential comes from (a secret, an env var on the worker).

## 2. Write the operation

One file per operation in the workflows crate under `src/operations/`. Template:

```rust,no_run
use std::future::Future;
use std::pin::Pin;

use ironflow_core::operations::http::Http;
use ironflow_core::error::OperationError;
use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;
use serde_json::{Value, json};

/// Posts a message to a Slack channel through an incoming webhook.
pub struct SlackMessage {
    pub webhook_url: String,
    pub text: String,
}

impl Operation for SlackMessage {
    fn kind(&self) -> &str {
        "slack"
    }

    fn input(&self) -> Option<Value> {
        // Persisted as the step input. Never include the credential.
        Some(json!({"text": self.text}))
    }

    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
        Box::pin(async move {
            let resp = Http::post(&self.webhook_url)
                .json(json!({"text": self.text}))
                .await?;
            if !resp.is_success() {
                return Err(EngineError::Operation(OperationError::Http {
                    status: Some(resp.status()),
                    message: format!("slack answered {}", resp.status()),
                }));
            }
            Ok(json!({"status": resp.status()}))
        })
    }
}
```

Two complete examples to copy from: `references/http-json.md` (generic JSON API wrapper with typed response) and `references/gitlab-issue.md` (create an issue, token from a secret).

## 3. Call it from a handler

```rust,no_run
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

struct Ping;

impl Operation for Ping {
    fn kind(&self) -> &str {
        "ping"
    }
    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
        Box::pin(async move { Ok(json!({"ok": true})) })
    }
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let out = ctx.operation("ping-upstream", &Ping).await?;
    let _ok = out.output["ok"].as_bool().unwrap_or(false);
    Ok(())
}
```

The step is stored with kind `custom:<kind()>`, so keep `kind()` short, lowercase and stable.

## 4. Verify

```bash
cargo build -p workflows
cargo test -p workflows
```

Add a test of the operation alone when it is pure enough (build the struct, call `execute().await`, assert on the JSON). For HTTP calls, test through a workflow with a real local server or leave it to the end-to-end test of the calling handler.

## Rules

- **Errors are `EngineError`.** Wrap transport errors as `EngineError::Operation(OperationError::Http { .. })`, bad configuration as `EngineError::StepConfig(..)`. `Http` errors convert with `?` already.
- **`input()` is observability, not secrets.** It lands in the database and the dashboard.
- **Output is JSON.** Return what later steps need (ids, urls, status), not the raw response body when it is large.
- **No retry loops inside `execute()`.** Retries are a step concern: the run fails, the operator retries the run.
- **Credentials from `ctx.secrets()` or the worker environment**, read in the handler and passed into the struct. `ctx.secrets()` needs `cargo add -p workflows ironflow-engine --features secret-store`.
