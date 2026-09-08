# Operations

An Operation is the extensibility mechanism for custom step types. Operations let you integrate external services (GitLab, Slack, any HTTP API) as tracked steps.

## The trait

```rust,ignore
use std::future::Future;
use std::pin::Pin;

use ironflow_engine::error::EngineError;
use serde_json::Value;

pub trait Operation: Send + Sync {
    fn kind(&self) -> &str;
    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>>;
    fn input(&self) -> Option<Value> { None }
}
```

- `kind()` returns a short identifier (e.g. `"slack"`, `"gitlab"`) stored in the database
- `execute()` runs the operation and returns JSON output
- `input()` optionally returns structured input for observability

## Using an operation in a workflow

Operations are invoked via `ctx.operation()`, which takes a step name and a reference to the operation:

```rust,ignore
use ironflow_engine::context::WorkflowContext;

let slack = SlackNotify::new(&webhook_url);
ctx.operation("notify-team", &slack).await?;
```

## Implementing an operation

```rust,ignore
use std::future::Future;
use std::pin::Pin;

use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;
use serde_json::{Value, json};

pub struct SlackNotify {
    webhook_url: String,
    message: String,
}

impl Operation for SlackNotify {
    fn kind(&self) -> &str {
        "slack-notify"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({ "message": self.message }))
    }

    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
        Box::pin(async move {
            // Send to Slack webhook using self.webhook_url
            Ok(json!({ "ok": true }))
        })
    }
}
```

## Built-in vs custom

Built-in step types (Shell, Http, Agent, Approval) have dedicated methods on `WorkflowContext`. Operations are for everything else -- they give you a typed extension point without modifying the engine.

See [Writing an Operation](../guides/writing-an-operation.md) for a step-by-step guide.
