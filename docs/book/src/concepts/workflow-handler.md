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
- **`sub_workflows()`** lists the handlers this one calls through `ctx.workflow`, for the call graph. Build it from the handlers, `sub_workflow_names(&[&Collect])`, never from hand-written names.

## Typed input for sub-workflows

A handler called as a sub-workflow declares its input type with `TypedWorkflow`.
`ctx.workflow(&Collect, CollectInput { .. })` then accepts nothing else, and
`input_schema()` is derived from the same type:

```rust,ignore
#[derive(Serialize, Deserialize, JsonSchema)]
struct CollectInput {
    host: String,
}

impl WorkflowHandler for Collect {
    fn name(&self) -> &str { "collect" }
    fn input_schema(&self) -> Option<Value> { Self::typed_input_schema() }
    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> { /* .. */ }
}

impl TypedWorkflow for Collect {
    type Input = CollectInput;
}

// In the parent:
let child = ctx.workflow(&Collect, CollectInput { host: "db-1".into() }).await?;
let steps = ctx.store().list_steps(child.run_id()).await?;
```

A child without input uses `type Input = ();`.

## Registration

Handlers are registered in the `Engine` before starting the server or worker:

```rust,ignore
let mut engine = Engine::new(store, provider);
engine.register(Box::new(Greeting))?;
```

See [Writing a Workflow](../guides/writing-a-workflow.md) for a step-by-step guide.
