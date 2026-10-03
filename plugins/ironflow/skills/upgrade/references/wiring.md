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

## rate-limit-peer-address
- kind: breaking
- since: ironflow-api after 2.43.5 (#152)
- detect: `\.into_make_service\(\)`
- compiler: `missing field `trusted_proxies` in initializer of `RouterConfig``

The rate limiters key on the TCP peer address and no longer believe `X-Forwarded-For` or
`X-Real-IP` from an arbitrary client. Serve the router with connect info, and pass the
trusted proxies through. Without connect info every client shares one bucket (a warning
is logged once).

```diff
+ use std::net::SocketAddr;
  let router_config = RouterConfig {
      dashboard_dir: config.dashboard_dir.clone(),
      rate_limit_auth: config.rate_limit_auth,
      rate_limit_general: config.rate_limit_general,
+     trusted_proxies: config.trusted_proxies.clone(),
      enforce_https: config.is_production,
  };
  let app = create_router(state, router_config)
      .layer(build_cors(&config))
-     .into_make_service();
+     .into_make_service_with_connect_info::<SocketAddr>();
```

Behind a reverse proxy (ingress, load balancer, nginx), set `TRUSTED_PROXIES` to its
addresses or CIDR ranges (`TRUSTED_PROXIES=10.0.0.0/8`), otherwise all users share the
proxy's bucket and hit `429` together. Tell the user before they deploy.

## sign-up-uniform-answer
- kind: behavior
- since: ironflow-api after 2.43.5 (#152)

`POST /auth/sign-up` answers `204` without session cookies, whether the email was free or
already registered: a client signs in next with the same credentials. `409
DUPLICATE_EMAIL` is gone from sign-up (a taken username still answers `409
DUPLICATE_USERNAME`). Passwords chosen at sign-up, at `PATCH /auth/password` and at
`POST /users` must pass `ironflow_auth::password::check_strength` (12 to 128 characters,
not common, not containing the email or username), otherwise `400 WEAK_PASSWORD`.
Existing passwords keep working at sign-in.

```diff
  client.post("/api/v1/auth/sign-up", &body).await?;
+ client.post("/api/v1/auth/sign-in", &credentials).await?;
```

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
