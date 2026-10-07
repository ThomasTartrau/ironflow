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

## Run priority

Every run carries a queue priority, an integer from -100 to 100 (default 0). When
a worker polls, the store hands out the pending run with the highest priority
first, then the oldest among equal priorities. A run whose `scheduled_at` is still
in the future is not eligible, whatever its priority, and concurrency groups
still apply: a blocked high priority run lets the next eligible one through.

A workflow declares the priority of its runs; the creation request can override
it:

```rust,ignore
impl WorkflowHandler for Hotfix {
    fn priority(&self) -> i16 {
        50
    }
    // ...
}

let options = EnqueueOptions {
    priority: Some(80),
    ..Default::default()
};
engine
    .enqueue_handler_with_options("hotfix", TriggerKind::Api, payload, options)
    .await?;
```

A handler priority outside the range is clamped; an explicit one is refused with
`EngineError::InvalidPriority`. `POST /api/v1/runs` takes `priority` in the body
(400 when out of range), `GET /api/v1/runs?priority=50` lists the runs of one
priority, and a schedule gives its own `priority` to every run it creates. A
retry or a replay keeps the priority of the original run, and a sub-workflow
takes `WorkflowOptions::priority`.

Priority only orders the queue:

- **No preemption.** A running run is never paused or evicted for a higher
  priority one; it keeps its worker slot until it finishes.
- **No aging.** A low priority run does not move up while it waits. A steady
  flow of higher priority runs can delay it indefinitely, so keep negative
  priorities for work that can wait.
## Routing runs to workers

A worker only takes the runs it can execute. When it polls, it sends the
workflows it registered and the tags it carries; the store hands out the oldest
pending run whose workflow is in that list and whose required tags are all
carried by the worker. Other runs stay `Pending` for a better-suited worker,
and the next one in the queue goes ahead.

Tags describe the host: hardware, region, network access.

```rust,ignore
let worker = WorkerBuilder::new(&api_url, &worker_token)
    .provider(Arc::new(ClaudeCodeProvider::new()))
    .register(Box::new(Transcode))
    .tags(["gpu", "region:eu"])
    .build()?;
```

A run requires the tags its handler declares with
[`required_worker_tags`](workflow-handler.md#worker-tags), plus the ones given
at creation: `EnqueueOptions::worker_tags`, `worker_tags` in
`POST /api/v1/runs`, `ironflow-cli run create --worker-tag gpu` or the
`worker_tags` argument of the MCP `create_run` tool. A tag is 1 to 64 ASCII
letters, digits or `- _ . : / =`, at most 32 per worker or run; `build()`
rejects an invalid one. Retrying a run keeps its tags; replaying it adds the
current handler's tags to the original ones.

A worker that sends no capabilities (released before this routing) still takes
every run, so workers can be upgraded one at a time. The
`ironflow_worker_queue_depth` gauge of a worker counts only the runs it can
take.

A run no worker can take waits in silence. While a run is `pending` or
`retrying`, `GET /api/v1/runs/{id}` returns `worker_routing`: the workers seen
recently by this API process and how many of them could take the run. The
dashboard and `ironflow-cli run get` warn when none was seen, or when none is
eligible.

Inside a worker, a `ctx.workflow(..)` step refuses a child whose required tags
the worker does not carry, since the child would run in its parent's slot on
the wrong host.

## Lease & Reaper

Workers hold a time-limited lease on each run they execute. If a worker crashes or is evicted, the lease expires and the Reaper (a background task in the API server) detects the orphaned run and requeues it.

A worker refreshes its lease every 30 seconds and the lease lasts 90 seconds, so three
missed refreshes make the run recoverable. A worker that cannot refresh its lease (the API
took the run away, or stayed unreachable longer than the lease) abandons the run instead of
executing it twice. Every 60 seconds the Reaper recovers at most 100 expired runs.

`AppState::spawn_background_tasks` starts the Reaper; a custom server that does not call it
must start one itself, or expired runs stay `Running`. Both sides can be tuned:

```rust,ignore
let reaper = Reaper::new(store, engine)
    .interval(Duration::from_secs(30))
    .batch_size(50);

let worker = WorkerBuilder::new(api_url, token)
    .worker_id("worker-eu-west-1a") // default: worker-<uuid>, new at every start
    .lease_ttl(Duration::from_secs(120))
    .lease_refresh_interval(Duration::from_secs(30))
    .build()?;
```

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

## Pausing runs and workflows

An administrator can hold a run, or a whole workflow, without cancelling
anything.

**A run.** `POST /api/v1/runs/{id}/pause` (`Engine::pause_run`) moves a root
run and every active sub-workflow run below it to `Paused`. Each run keeps the
state it was paused from in `resume_status`:

- a queued (`Pending`, `Retrying`) run is no longer picked by a worker;
- a `Sleeping` run is no longer woken by the Waker;
- a `Running` run has its step in flight interrupted at once: the step is
  marked `Failed` with `interrupted: worker lease lost` and no further step
  is started. A worker loses the lease of the run, and the reaper never
  touches a paused run;
- an approval, a human input or a signal the run waits for can still be
  resolved while it is paused. The decision is recorded and only changes the
  state the run resumes to; a rejection fails the run.

`POST /api/v1/runs/{id}/resume` (`Engine::resume_paused_run`) puts every run
back in the state it was paused from. A run paused while it executed goes
back to `Pending` and replays: finished steps are skipped and the interrupted
step is executed again, like a run resumed after a lost lease. A `Sleeping`
run whose deadline passed during the pause goes back to `Pending`. A paused
run can also be cancelled.

Only a root run is paused or resumed: the API answers 400 for a sub-workflow
run, a run already finished, or (on resume) a run that is not paused.

**A workflow.** `POST /api/v1/workflows/{name}/pause` (`Engine::pause_workflow`)
records a pause for the workflow: runs are still created, but workers leave
them queued until `POST /api/v1/workflows/{name}/resume`. Runs already
executing are not affected; pause them one by one. `GET /api/v1/workflows`
and `GET /api/v1/workflows/{name}` report `paused_at` while the workflow is
paused.

The CLI exposes the same actions as `ironflow-cli run pause|resume <id>` and
`ironflow-cli workflow pause|resume <name>`, the MCP server as the `pause_run`,
`resume_run`, `pause_workflow` and `resume_workflow` tools.

## Scaling

Workers are stateless. Add more workers to increase throughput. Each worker polls independently -- no coordination is needed beyond the API server.
