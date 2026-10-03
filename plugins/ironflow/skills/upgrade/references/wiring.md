# Wiring

Changes outside the handlers: server and worker binaries, environment, deployment. The
reference wiring is the setup template, `setup/assets/server/src/main.rs` and
`setup/assets/worker/src/main.rs`, built against the latest release in Ironflow's CI.

## server-secrets
- kind: behavior
- since: ironflow-api 2.35.5 (#87), ironflow-api after 2.43.1 (#149)
- detect: `^[^#/]*ironflow-dev-`

The server refuses to boot unless `WORKER_TOKEN` and `JWT_SECRET` are set, 32 bytes or
more, without the `ironflow-dev-` prefix. Only `IRONFLOW_ENV=development` allows them to be
missing (generated per process). A worker that falls back to the published dev token no
longer authenticates.

```diff
- let token = env::var("WORKER_TOKEN").unwrap_or_else(|_| "ironflow-dev-worker-token".into());
+ let token = env::var("WORKER_TOKEN").expect("WORKER_TOKEN must be set");
```

Check every place the server runs (`.env`, compose files, Helm values, CI): each needs both
secrets, generated with `openssl rand -hex 32`, the server and its workers sharing the same
`WORKER_TOKEN`. Tell the user before they deploy: it is a boot failure, not a compile error.

## run-purger
- kind: behavior
- since: ironflow-api 2.43.0 (#148)
- detect: `ServerConfig::from_env`

A custom server must start the purger itself, otherwise `PURGE_MAX_AGE_DAYS`,
`PURGE_MAX_RUNS_PER_WORKFLOW`, `PURGE_DRY_RUN`, `PURGE_INTERVAL_SECS`,
`PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS` and `SIGNAL_RETENTION_DAYS` are ignored and old runs
pile up. Applies only if the server's `main.rs` has no `RunPurger`.

```diff
+ use ironflow_api::purger::RunPurger;
  let shutdown = state.spawn_background_tasks().await;
+ tokio::spawn(
+     RunPurger::from_config(store.clone(), &config)
+         .with_blob_store(state.blob_store.clone())
+         .run(shutdown.clone()),
+ );
```

## runtime-cron
- kind: breaking
- since: ironflow-runtime 2.4.25 (#77)
- detect: `\.cron\("|run_crons\(|ironflow_runtime::cron`
- compiler: `no method named cron found for struct Runtime`

Cron jobs left `ironflow-runtime`. A scheduled job becomes a workflow that declares its
schedule; the API server's schedule ticker starts the runs, visible and retried like any
other run.

```diff
- Runtime::new().cron("0 0 * * * *", "hourly-sync", || async { sync().await })
+ static HOURLY: LazyLock<CronSchedule> =
+     LazyLock::new(|| CronSchedule::new("0 0 * * * *").expect("valid cron expression"));
+
+ impl WorkflowHandler for HourlySync {
+     fn name(&self) -> &str { "hourly-sync" }
+     fn schedule(&self) -> Option<&CronSchedule> { Some(&HOURLY) }
+     fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> { .. }
+ }
```

`use ironflow_engine::schedule::CronSchedule;`. Register the handler in `handlers()`. The
body of the old job becomes steps (see the workflow skill).

## decision-provider
- kind: behavior
- since: ironflow-worker 2.21.0 (#101)
- detect: `\.decision\("`

A workflow with a `ctx.decision` step fails with `NoDecisionProvider` on a worker that has
none. Check the worker's `main.rs` calls `WorkerBuilder::decision_provider(..)`; tests use
`Engine::with_decision_provider(..)`.

## web-fetch-internal-hosts
- kind: behavior
- since: ironflow-core after 4.8.0 (#155)

The `web_fetch` agent tool refuses a URL that is, resolves to, or redirects to a private,
loopback, link-local or cloud metadata address, including decimal, hex, octal and
IPv4-mapped forms; proxy variables are ignored. An agent that read an internal page now
gets a tool error. Allow each internal host the agent must read where the worker registers
the tool:

```diff
- .register(WebFetchTool::new())
+ .register(WebFetchTool::new().allow_host("docs.internal"))
```
