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

## worker-routing
- kind: behavior
- since: ironflow-store after 2.44.0, ironflow-engine after 2.49.0, ironflow-api after 2.47.0, ironflow-worker after 2.24.0 (#170)

A worker now sends the workflows it registered and the tags it carries when it polls, and
only takes the runs it can execute: a run of a workflow it did not register stays
`pending` for another worker instead of being picked here. Deploy every worker binary
with the full `handlers()` list, or the runs of a workflow no worker registers stay
`pending` (the dashboard and `ironflow-cli run get` warn about them). A workflow that must
run on a given host declares `required_worker_tags`, and only the workers built with all
of those tags take it:

```diff
  let mut builder = WorkerBuilder::new(&api_url, &worker_token)
      .provider(provider)
+     .tags(["gpu", "region:eu"])
      .concurrency(concurrency)
```

A worker released before this change sends no capabilities and still takes every run, so
workers can be upgraded one at a time. Upgrade the API server first: an older server
ignores the filter and hands any run to an upgraded worker.

## rate-limit-client-ip-header
- kind: behavior
- since: ironflow-api after 2.52.1 (#198)
- detect: `TRUSTED_PROXIES`

Behind a trusted proxy the rate limiters read one forwarding header only, chosen by
`TRUSTED_PROXY_HEADER`: `x-forwarded-for` (default) or `x-real-ip`. `X-Real-IP` is no
longer a fallback when `X-Forwarded-For` is missing, so behind a proxy that only sets
`X-Real-IP` the client IP is the proxy itself: every client shares the proxy's single
bucket, and users lock each other out, until the header is configured. A server served
without connect info no longer puts every client in one shared bucket: its IP limit is
skipped, with a warning logged once, so serve it with
`into_make_service_with_connect_info::<SocketAddr>()`.

```diff
  TRUSTED_PROXIES=10.0.0.0/8
+ TRUSTED_PROXY_HEADER=x-real-ip
```

## api-key-rate-limit-override-admin
- kind: behavior
- since: ironflow-api after 2.52.1 (#198)
- detect: `rate_limit_override`

Only an admin may set `rate_limit_override` on an API key: a member gets `403`. An override
already stored on a key owned by a member is ignored, and the key uses the server-wide
limit. A request is counted under an API key only once its secret is verified: a token
whose secret does not verify counts on the IP bucket, and a client guessing secrets of one
key gets `429` after 10 verifications a minute.

```diff
- ironflow-cli api-key create --name ci --scope runs_read --rate-limit-override 0
+ ironflow-cli api-key create --name ci --scope runs_read
```

## audit-log-subscriber
- kind: behavior
- since: ironflow-engine after 2.52.1, ironflow-api after 2.52.3 (#202)

The audit log (`GET /api/v1/audit-logs`, the Audit page of the dashboard) is only filled
when the server subscribes `AuditLogSubscriber` to the engine; without it the list stays
empty. Approval decisions now carry the caller's `user_id` (the key owner for an API key),
and the pause and resume of a workflow and the rejection of a human input are recorded
too. Subscribe it next to the SSE broadcaster:

```diff
- use ironflow_engine::notify::{Event, WebhookSubscriber, WorkflowEventBus};
+ use ironflow_engine::notify::{AuditLogSubscriber, Event, WebhookSubscriber, WorkflowEventBus};
+ engine.subscribe(AuditLogSubscriber::new(store.clone()), Event::ALL);
  let sse_broadcaster = SseBroadcaster::new();
```

Log lines and Provider Account events do not go through the engine publisher, so
`Event::ALL` neither floods the audit log nor duplicates them.

## resume-workflow-actor
- kind: breaking
- since: ironflow-engine after 2.52.1 (#202)
- detect: `engine\.resume_workflow\([^,)]*\)`
- compiler: `this method takes 2 arguments but 1 argument was supplied`

`Engine::resume_workflow` takes the user who resumes the workflow, recorded in the
`workflow_resumed` audit event. Pass `None` outside a user context:

```diff
- engine.resume_workflow("deploy").await?;
+ engine.resume_workflow("deploy", None).await?;
```
