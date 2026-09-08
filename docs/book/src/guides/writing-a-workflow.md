# Writing a Workflow

This guide walks through creating a workflow handler from scratch.

## 1. Define your input

If your workflow accepts input, define a struct with `Deserialize` and `JsonSchema`:

```rust,ignore
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Deserialize, JsonSchema)]
struct DeployInput {
    environment: String,
    version: String,
}
```

The `JsonSchema` derive lets the dashboard render a dynamic form for triggering the workflow.

## 2. Implement WorkflowHandler

```rust,ignore
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler, input_schema_for};
use serde_json::Value;

pub struct Deploy;

impl WorkflowHandler for Deploy {
    fn name(&self) -> &str {
        "deploy"
    }

    fn description(&self) -> &str {
        "Deploy a version to an environment"
    }

    fn input_schema(&self) -> Option<Value> {
        Some(input_schema_for::<DeployInput>())
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            let input: DeployInput = ctx.input().await?;

            ctx.shell(
                "build",
                ShellConfig::new(&format!("echo 'Building {}'", input.version)),
            ).await?;

            ctx.shell(
                "deploy",
                ShellConfig::new(&format!(
                    "echo 'Deploying {} to {}'",
                    input.version, input.environment
                )),
            ).await?;

            Ok(())
        })
    }
}
```

## 3. Register in your handlers list

```rust,ignore
pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {
    vec![
        Box::new(Deploy),
        // ... other handlers
    ]
}
```

Both the server and the worker must register the same handlers. The recommended pattern is a shared `handlers()` function in a library crate.

## 4. Complete example

The greeting workflow in the examples directory demonstrates all features:

```rust,ignore
{{#include ../../../../examples/ironflow-workflows/src/greeting.rs}}
```
