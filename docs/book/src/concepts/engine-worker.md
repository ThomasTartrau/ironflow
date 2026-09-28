# Engine & Worker

## Engine

The Engine is the in-memory registry that maps workflow names to handlers and orchestrates run execution. It holds references to the Store (persistence), the Provider (agent backends), and the event publisher.

```rust,ignore
let mut engine = Engine::new(store, provider);
engine.register(Box::new(MyWorkflow))?;
```

The Engine is used by both the API server (for metadata and describe endpoints) and the Worker (for execution).

Before running an agent step, the engine stamps the `ironflow.io/run-id`,
`ironflow.io/root-run-id` and `ironflow.io/step` pod labels on its config (the
step name is sanitized into a valid label value), so the Kubernetes providers
can tag the pod and clean up a previous attempt of the same step on retry. The
root run is the run itself, or the top-level run inside a sub-workflow
(`ctx.root_run_id()`).

Before every execution of a run (`Engine::execute_handler_run`, the first one
included, and `Engine::resume_run` after a gate under `ExecutionMode::Local`),
the engine calls `AgentProvider::release_run` with the run id. The
default does nothing; `K8sEphemeralProvider` deletes the pods left by a dead
attempt of the run or of its sub-workflows, and waits until they are gone. A
failed release fails the execution with a replayable error: the run goes to
`Retrying` while it has retries left.

## Execution mode

A run suspended on an approval, a human input or an escalation resumes once the
gate is resolved. `Engine::with_execution_mode` decides where that happens:

- `ExecutionMode::Local` (default): the API process moves the run to `Running`
  and calls `Engine::resume_run` itself. Use it for single-process deployments
  where the API also registers the handlers. `TestEngine` always resumes this way.
- `ExecutionMode::Workers`: the API moves the run back to `Pending`. A worker
  claims it through `pick_next_pending` and finishes it with
  `Engine::execute_handler_run`, replaying the steps that already completed.
  Use it when the API runs without the workspace, tools or handlers the workflow
  needs.

```rust,ignore
let engine = Engine::new(store, provider)
    .with_execution_mode(ExecutionMode::Workers);
```

## Worker

A Worker is a background process that:

1. Polls the API for pending runs
2. Acquires a lease on a run
3. Executes the workflow handler via the Engine
4. Refreshes the lease periodically during execution
5. Reports the result back to the API

```rust,ignore
let worker = WorkerBuilder::new(&api_url, &worker_token)
    .provider(Arc::new(ClaudeCodeProvider::new()))
    .concurrency(2)
    .poll_interval(Duration::from_secs(2))
    .register(Box::new(MyWorkflow))
    .build()?;

worker.run().await?;
```

Workflows that use `ctx.decision(...)` need a decision provider on the worker too:
`.decision_provider(Arc::new(TypeSafeProvider::new(api_key)))`. See
[Decisions](decision.md).

## Lease & Reaper

Workers hold a time-limited lease on each run they execute. If a worker crashes or is evicted, the lease expires and the Reaper (a background task in the API server) detects the orphaned run and requeues it.

## Scaling

Workers are stateless. Add more workers to increase throughput. Each worker polls independently -- no coordination is needed beyond the API server.
