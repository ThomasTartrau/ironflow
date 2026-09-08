# Engine & Worker

## Engine

The Engine is the in-memory registry that maps workflow names to handlers and orchestrates run execution. It holds references to the Store (persistence), the Provider (agent backends), and the event publisher.

```rust,ignore
let mut engine = Engine::new(store, provider);
engine.register(Box::new(MyWorkflow))?;
```

The Engine is used by both the API server (for metadata and describe endpoints) and the Worker (for execution).

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

## Lease & Reaper

Workers hold a time-limited lease on each run they execute. If a worker crashes or is evicted, the lease expires and the Reaper (a background task in the API server) detects the orphaned run and requeues it.

## Scaling

Workers are stateless. Add more workers to increase throughput. Each worker polls independently -- no coordination is needed beyond the API server.
