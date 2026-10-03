# Running the Server

The API server exposes a REST API for managing workflows, runs, and steps. It also serves the web dashboard.

## Example server

The repository includes a complete example server:

```rust,ignore
{{#include ../../../../examples/server/src/main.rs}}
```

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `IRONFLOW_ENV` | `development` | `production` or `development` |
| `IRONFLOW_INSECURE_COOKIES` | `false` | `1` or `true` drops the `Secure` flag from session cookies. Development only, ignored in production |
| `DATABASE_URL` | -- | PostgreSQL URL (required in production) |
| `JWT_SECRET` | dev secret | JWT signing key (**in production: mandatory, >= 32 bytes, must not start with `ironflow-dev-`**) |
| `WORKER_TOKEN` | dev token | Shared secret for worker auth (**in production: mandatory, >= 32 bytes, must not start with `ironflow-dev-`**) |
| `PORT` | `3000` | HTTP listen port |
| `ALLOWED_ORIGINS` | same-origin | Comma-separated CORS origins |
| `ARTIFACTS_DIR` | -- | Filesystem root for step artifacts |
| `PURGE_MAX_AGE_DAYS` | `90` | Terminal runs older than this are purged |
| `PURGE_MAX_RUNS_PER_WORKFLOW` | `1000` | Terminal runs kept per workflow |
| `PURGE_DRY_RUN` | `false` | Log what would be purged without deleting (also disables usage and signal purging) |
| `PURGE_INTERVAL_SECS` | `86400` | Seconds between purger ticks (min 60) |
| `PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS` | `30` | Days of Provider Account usage history kept (min 1) |
| `SIGNAL_RETENTION_DAYS` | `7` | Days received [signals](../concepts/signals.md) are kept before the purger deletes them (min 1) |

### Transport security

Session cookies are `HttpOnly`, `SameSite=Lax` and `Secure`. Browsers treat
`http://localhost` as a secure context, so `Secure` cookies work in local development. For
a plain-HTTP setup on another origin, set `IRONFLOW_INSECURE_COOKIES=1`; the flag is
ignored when `IRONFLOW_ENV=production`.

In production the example server also sets `RouterConfig::enforce_https`: TLS terminates at
your reverse proxy, and any request that arrives with `X-Forwarded-Proto: http` is answered
with a `308 Permanent Redirect` to `https://<host><path>` (the host comes from
`X-Forwarded-Host`, then `Host`). The method and body are preserved. Requests without
`X-Forwarded-Proto`, such as in-cluster health probes and worker traffic, are not
redirected. Every response carries `Strict-Transport-Security`.

The reverse proxy must therefore set `X-Forwarded-Proto` (and `X-Forwarded-Host` if it
rewrites `Host`), and must overwrite any value sent by the client.

### Retention

The example server starts a `RunPurger` built from these variables: it purges old runs,
Provider Account usage history and received signals. A custom server must start it
itself with `RunPurger::from_config`, otherwise these variables are ignored:

```rust,ignore
let shutdown = state.spawn_background_tasks().await;
tokio::spawn(
    RunPurger::from_config(store.clone(), &config)
        .with_blob_store(state.blob_store.clone())
        .run(shutdown.clone()),
);
```

## Running

```sh
cargo run -p ironflow-example-server
```

The server starts on `http://localhost:3000`. The dashboard is available at the root URL.
