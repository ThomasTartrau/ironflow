# WorkflowHandler

A `WorkflowHandler` is the core abstraction in Ironflow. It defines a named workflow as imperative Rust code.

## The trait

```rust,ignore
pub trait WorkflowHandler: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a>;

    // Optional methods
    fn category(&self) -> Option<&str> { None }
    fn input_schema(&self) -> Option<Value> { None }
    fn default_labels(&self) -> HashMap<String, String> { HashMap::new() }
    fn source_code(&self) -> Option<&str> { None }
}
```

## Example: a greeting workflow

```rust,ignore
{{#include ../../../../examples/ironflow-workflows/src/greeting.rs}}
```

## Key points

- **`name()`** must be unique across all registered handlers. It identifies the workflow in the API and the database.
- **`execute()`** receives a `WorkflowContext` to create steps. Steps are persisted as they complete.
- **`input_schema()`** returns a JSON Schema derived from a `#[derive(JsonSchema)]` struct. The dashboard renders it as a dynamic form.
- **`source_code()`** optionally embeds the handler source for display in the dashboard.

## Registration

Handlers are registered in the `Engine` before starting the server or worker:

```rust,ignore
let mut engine = Engine::new(store, provider);
engine.register(Box::new(Greeting))?;
```

See [Writing a Workflow](../guides/writing-a-workflow.md) for a step-by-step guide.
