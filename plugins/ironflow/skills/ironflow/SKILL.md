---
name: ironflow
description: Entry point for building on the Ironflow workflow engine. Use for setting up an Ironflow project (server + worker), writing a WorkflowHandler, writing a custom Operation, testing a workflow end to end, or reviewing a handler. Trigger on "ironflow", "workflow handler", "ironflow setup", "ctx.shell", "ctx.agent", "approval gate", "créer un workflow ironflow", "mettre en place ironflow".
argument-hint: "[setup|workflow|operation|test|review] [name]"
---

# Ironflow

Routes to one sub-skill per verb. Arguments: `$ARGUMENTS`.

## Route

| Verb | Load | Purpose |
|---|---|---|
| `setup` | skill `ironflow:setup` | Scaffold a workspace (workflows lib, server, worker, hello workflow, e2e test) |
| `workflow <name>` | skill `ironflow:workflow` | Write a `WorkflowHandler` and register it |
| `operation <name>` | skill `ironflow:operation` | Write a custom `Operation` (API call tracked as a step) |
| `test <workflow>` | skill `ironflow:test` | Write an end-to-end test for a handler |
| `review [file]` | agent `ironflow:workflow-reviewer` | Review a handler for replay, naming and secret pitfalls |

Invoke the sub-skill with the Skill tool, passing the remaining arguments. For `review`, spawn the agent with the file path (or the handler name to locate).

## No verb given

Detect the project state before asking anything:

```bash
# Is this an ironflow project?
grep -rl "ironflow-engine" --include=Cargo.toml . 2>/dev/null | head -3
# Which handlers exist?
grep -rn "impl WorkflowHandler for" --include=*.rs . 2>/dev/null | head -20
```

- No `ironflow-engine` dependency anywhere: propose `setup`. Say what it creates in one line.
- Dependency present: list the handlers found and propose `workflow`, `operation`, `test` or `review`. One question, not a menu of everything.

## Conventions shared by every sub-skill

- Never write a crate version by hand. Add dependencies with `cargo add` so the project picks the latest release.
- Imports at the top of the file with `use`. No inline `some::path::Type` in bodies.
- Every handler has a typed input struct deriving `serde::Deserialize` and `schemars::JsonSchema`, exposed through `input_schema()`, read with `ctx.input::<T>()`.
- Every handler is added to `handlers()` in the workflows crate. That one list feeds the server and the worker.
- After `workflow` finishes, offer the review: "Run the workflow reviewer on this handler?" Do not run it without asking.
