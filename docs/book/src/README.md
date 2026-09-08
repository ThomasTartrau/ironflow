# Ironflow

Ironflow is a workflow orchestration platform where workflows are imperative Rust code executed by background workers, with persistence, cost tracking, and human approval gates.

## Why Ironflow?

- **Workflows are Rust code.** No YAML, no DSL. Full type safety, IDE support, and compile-time checks.
- **Persistent execution.** Every step is tracked in a database. Runs survive process restarts.
- **Human-in-the-loop.** Approval gates pause a run until someone approves or rejects it.
- **Cost tracking.** Every agent call is metered. Set per-run and monthly budgets.
- **Scalable.** Add more workers to increase throughput. Workers poll the API for pending runs.

## Quick links

- [Getting Started](getting-started/installation.md) -- install, configure, run
- [Concepts](concepts/workflow-handler.md) -- understand the building blocks
- [Guides](guides/writing-a-workflow.md) -- step-by-step tutorials
- [Architecture](architecture/overview.md) -- how the pieces fit together
- [API Reference (docs.rs)](https://docs.rs/ironflow-engine) -- generated from rustdoc
