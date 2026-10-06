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

1. Waits for a free execution slot (`concurrency`)
2. Polls the API for a pending run and acquires a lease on it
3. Executes the workflow handler via the Engine
4. Refreshes the lease periodically during execution
5. Reports the result back to the API

A saturated worker does not poll: a run is only claimed once a slot can execute
it, so its lease never expires while it waits.

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

When [Provider Accounts](provider-accounts.md) exist, the worker picks one for
every agent step and injects its credential; `WorkerBuilder::account_strategy`
chooses how. When every account is limited the run sleeps until the next
reset, up to `WorkerBuilder::max_capacity_wait` (6 hours by default), and the
step runs again when it wakes.

## Concurrency groups

Worker `concurrency` caps one process. To cap the runs that share an external
resource across every worker (a repository, a tenant, an environment), give the
run concurrency groups when it is created:

```rust,ignore
let options = EnqueueOptions {
    concurrency_limits: vec![ConcurrencyLimit::new("repo:acme", 2)],
    ..Default::default()
};
engine
    .enqueue_handler_with_options("fix-issue", TriggerKind::Api, payload, options)
    .await?;
```

`POST /api/v1/runs` takes the same list as `concurrency_limits`. When a worker
polls, the store only hands out a run whose groups all count fewer running root
runs than the run's limit. A blocked run stays `Pending` and the worker takes the
next free one, so one saturated group never stalls the others. Sub-workflow runs
execute inside their parent's slot and are not counted. The
`ironflow_worker_queue_blocked_runs{group}` gauge reports the pending runs each
saturated group holds back.

## Lease & Reaper

Workers hold a time-limited lease on each run they execute. If a worker crashes or is evicted, the lease expires and the Reaper (a background task in the API server) detects the orphaned run and requeues it.

A sub-workflow child that was suspended (human input, signal, delay) is picked
like any run, but the worker resumes its root run instead. The lease follows:
the root takes the child's lease when it goes back to `Running`, the child
releases it, and the worker keeps refreshing the root until it finishes. A
root resumed this way is recovered by the Reaper like any other run.

### Resuming after a lost lease

A requeued run resumes where its worker stopped, in the same attempt:

- Steps that completed, were skipped, or got their approval or human input are
  replayed from the store. They are not executed again, and nobody is asked
  twice.
- A step that was running when the worker died is marked `Failed` with
  `interrupted: worker lease lost`. The next worker executes it again at the
  same position, and the interrupted record stays in the step history.
  `Pending` and `AwaitingApproval` steps are left as they are.
- An interrupted `ctx.workflow` step re-enters the child run it had started
  instead of starting a new one. The child's running steps are interrupted
  the same way, and its finished steps are replayed.
- An interrupted agent step resumes the Claude Code session it was running
  in, with a resume prompt, instead of starting the agent from scratch. See
  [Resuming an interrupted agent step](../guides/transports.md#resuming-an-interrupted-agent-step).

The interrupted step really runs twice, so make it idempotent: a deploy, a
payment or a notification must tolerate a second call.

Lease recoveries are counted in `lease_recoveries`, apart from `retry_count`.
A run can be recovered `max_retries` times. One more lost lease fails the run
with `worker lease expired`, and its open steps are failed with the same error;
the sub-workflow runs it left running are cancelled. A requeued run keeps its
children: it re-enters them when it resumes.
A handler failure still uses `retry_count` and starts a new attempt that
replays nothing. A run whose handler changed to an incompatible version since
it was created replays nothing either and fails with `HANDLER_VERSION_MISMATCH`.

## Waker

Runs paused in `Sleeping` (a `ctx.delay` step, a `ctx.wait_for_signal` step waiting for its signal, or an agent step waiting for [provider capacity](provider-accounts.md#when-every-account-is-limited)) carry their wake-up time in `scheduled_at`. The Waker, a background task of the API server, claims every due run every 10 seconds and moves it back to `Pending` exactly once, even with several API instances. Under `ExecutionMode::Local` the API then resumes the run in-process; under `ExecutionMode::Workers` a worker picks it up. A delivered [signal](signals.md) wakes its runs right away, without waiting for the Waker.

## Scaling

Workers are stateless. Add more workers to increase throughput. Each worker polls independently -- no coordination is needed beyond the API server.
