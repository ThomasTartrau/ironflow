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
| `IRONFLOW_ENV` | unset | `production`, or `development` to boot without secrets |
| `IRONFLOW_INSECURE_COOKIES` | `false` | `1` or `true` drops the `Secure` flag from session cookies. Development only, ignored in production |
| `DATABASE_URL` | -- | PostgreSQL URL (required in production) |
| `JWT_SECRET` | -- | JWT signing key (**mandatory, >= 32 bytes, must not start with `ironflow-dev-`**; with `IRONFLOW_ENV=development`: random per process if unset, no minimum length) |
| `WORKER_TOKEN` | -- | Shared secret for worker auth (same rules as `JWT_SECRET`; a generated one is logged at startup so a worker can use it) |
| `PORT` | `3000` | HTTP listen port |
| `ALLOWED_ORIGINS` | same-origin | Comma-separated CORS origins |
| `RATE_LIMIT_AUTH` | `10` | Sign-in / sign-up / password change requests per minute, per caller (client IP, or user for a password change) and per targeted email (`0` disables) |
| `RATE_LIMIT_GENERAL` | `60` | Other API requests per minute, per caller (`0` disables) |
| `SSE_MAX_CONNECTIONS_PER_PRINCIPAL` | `5` | Simultaneous SSE streams (`/events`, `/runs/{id}/events`) per user or API key owner; beyond it the answer is `429` (`0` disables). A dashboard tab holds up to two |
| `SSE_MAX_CONNECTIONS` | `1000` | Same cap for the whole server (`0` disables) |
| `TRUSTED_PROXIES` | -- | Comma-separated IPs or CIDR ranges of the reverse proxies whose forwarding header is believed |
| `TRUSTED_PROXY_HEADER` | `x-forwarded-for` | Header a trusted proxy reports the client IP in: `x-forwarded-for`, or `x-real-ip` to read only that header. There is no fallback between the two |
| `ARTIFACTS_DIR` | -- | Filesystem root for step [artifacts](../concepts/artifacts.md); unset disables them |
| `ARTIFACT_MAX_BYTES` | `104857600` | Maximum size of one artifact (100 MiB) |
| `IRONFLOW_SECRET_KEYS` | -- | Versioned AES-GCM keys for the secret store, see [Secret encryption keys](#secret-encryption-keys); unset disables secrets |
| `IRONFLOW_SECRET_ACTIVE_KEY_VERSION` | highest configured | Key version used to encrypt new secrets |
| `IRONFLOW_SECRET_KEY` | -- | Deprecated single key, read as version 1 |
| `DASHBOARD_DIR` | embedded | Serve the dashboard from this directory instead of the embedded copy |
| `WEBHOOK_URL` | -- | Outbound webhook notified of run events |
| `PURGE_MAX_AGE_DAYS` | `90` | Terminal runs older than this are purged |
| `PURGE_MAX_RUNS_PER_WORKFLOW` | `1000` | Terminal runs kept per workflow |
| `PURGE_DRY_RUN` | `false` | Log what would be purged without deleting (also disables usage and signal purging) |
| `PURGE_INTERVAL_SECS` | `86400` | Seconds between purger ticks (min 60) |
| `PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS` | `30` | Days of Provider Account usage history kept (min 1) |
| `SIGNAL_RETENTION_DAYS` | `7` | Days received [signals](../concepts/signals.md) are kept before the purger deletes them (min 1) |

No secret is built into the binary. `JWT_SECRET` and `WORKER_TOKEN` must be at least 32
bytes and must not start with `ironflow-dev-`; generate them with `openssl rand -hex 32`.
Only `IRONFLOW_ENV=development` boots without them. Production also requires
`DATABASE_URL`. Any violation aborts at boot with every error listed.

### Secret encryption keys

`IRONFLOW_SECRET_KEYS` holds one or more versioned AES-GCM keys, as `version:hex` pairs
separated by commas. Each key is 64 hex characters (32 bytes):

```sh
IRONFLOW_SECRET_KEYS="1:0123...ef,2:fedc...10"
IRONFLOW_SECRET_ACTIVE_KEY_VERSION=2
```

Every key in the ring can decrypt; only the active one encrypts. Without any key the secret
store stays off and workflows reading secrets fail.

`IRONFLOW_SECRET_KEY` (a single unversioned key) is still accepted and read as version 1,
so existing deployments keep working. It is deprecated: when `IRONFLOW_SECRET_KEYS` is also
set, it is ignored with a warning.

The server refuses to start if a stored secret uses a key version absent from the
configuration, and names the missing versions. That is the safety net behind the rotation
procedure below.

#### Rotating the encryption key

```sh
# 1. Add the new key without activating it, then restart.
#    New secrets stay on version 1; version 2 is merely available.
IRONFLOW_SECRET_KEYS="1:<hexA>,2:<hexB>"
IRONFLOW_SECRET_ACTIVE_KEY_VERSION=1

# 2. Activate version 2, then restart.
#    New secrets use version 2; older ones stay readable.
IRONFLOW_SECRET_ACTIVE_KEY_VERSION=2

# 3. Re-encrypt the existing secrets.
ironflow-cli secret rotate

# 4. Confirm version 1 is no longer used by any secret.
ironflow-cli secret key-status

# 5. Drop version 1, then restart.
IRONFLOW_SECRET_KEYS="2:<hexB>"
```

Step 1 is kept separate from step 2 so rolling back to the previous deployment stays
possible for as long as nothing has been encrypted with the new key.

`secret rotate` works in batches and is safe to interrupt: secrets already re-encrypted are
skipped on the next run, and every secret stays readable throughout. It re-encrypts in place:
the ID, key and timestamps of a secret never change. A secret that cannot be decrypted is
skipped and reported, and the command exits non-zero.

`secret key-status` reports which versions are configured, which are used by stored
secrets, and which can be retired.

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

### Rate limiting and client IP

An unauthenticated request is rate limited under its client IP: the address of the TCP
peer. Serve the router with `into_make_service_with_connect_info::<SocketAddr>()`, as the
example server does; without it the IP limit is skipped and a warning is logged once.

Forwarding headers are written by the client, so they are ignored unless the peer is listed
in `TRUSTED_PROXIES`. Behind a reverse proxy, list it (`TRUSTED_PROXIES=10.0.0.0/8`). Only
the header named by `TRUSTED_PROXY_HEADER` is read then. With the default,
`x-forwarded-for`, `X-Forwarded-For` is read from the right, trusted hops are skipped and
the first other address is the client. With `x-real-ip`, `X-Real-IP` is the client. The
other header is never read, not even as a fallback. Without `TRUSTED_PROXIES`, every user
shares the proxy's bucket.

A request with an API key is counted under that key only once its secret is verified.
A secret that does not verify counts on the IP bucket. Unverified secrets are checked at
most 10 times per minute per client and key, then answered `429`. Only an admin may set
an API key's `rate_limit_override` (a member gets `403`), and it is honoured only for keys
owned by an admin.

Sign-in and sign-up are also counted per targeted email, so attempts on one account spread
over many addresses are limited too. The two routes have separate counters: a sign-up flood
does not lock the account out of sign-in.

Password change (`PATCH /api/v1/auth/password`) sits behind the same limiter, counted per
signed-in user: guesses at the current password beyond `RATE_LIMIT_AUTH` per minute answer
`429` before the password is checked, and each wrong current password is logged with the
user id.

### Accounts and passwords

A password chosen at sign-up, password change or admin user creation must be 12 to 128
characters, not a common password (nor one padded with digits and symbols, like
`Password2024!`), must not contain the email or username, and must not be repetitive.
Otherwise the request answers `400 WEAK_PASSWORD` with the rule it broke.

Sign-up answers `204` without a session, whether the email was free or already
registered: the dashboard signs in next with the same credentials. A taken username still
answers `409 DUPLICATE_USERNAME`.

The first account created on an empty database becomes admin. The decision is atomic:
concurrent sign-ups on a fresh install cannot each see an empty table, so exactly one of them
is admin.

Sign-in on an unknown email runs one Argon2 verification against a throwaway hash, like a
wrong password, so the answer time does not tell whether the email has an account. Both
answer `401 INVALID_CREDENTIALS` and are logged without the email.

Sign-up has no email verification, so it still tells whether an email has an account: a
sign-up followed by a sign-in with the chosen password succeeds only if the email was free.
When email enumeration matters, build the server without the `sign-up` feature in
production and create accounts as an admin, or keep sign-up behind an invite flow.

### API key scopes

A request authenticated with an API key (`Authorization: Bearer irfl_...`) must hold the
scope of the route, otherwise it answers `403 INSUFFICIENT_SCOPE` before any lookup, so
a key without the scope cannot tell whether a run or a secret exists. The `admin` scope
implies every other scope. Paths below are relative to `/api/v1`.

| Scope | Routes |
|-------|--------|
| `runs_read` | `GET /runs`, `GET /runs/{id}`, `GET /runs/{id}/logs`, `GET /runs/{id}/events`, `GET /runs/{id}/steps/{step_id}/artifacts/{name}`, `GET /events` |
| `runs_write` | `POST /runs` |
| `runs_manage` | `POST /runs/{id}/approve`, `/reject`, `/cancel`, `/retry`, `/replay`, `/pause`, `/resume`, `POST /runs/{id}/steps/{step_id}/input`, `/reject` |
| `workflows_read` | `GET /workflows`, `GET /workflows/{name}`, `POST /workflows/{name}/plan` |
| `stats_read` | `GET /stats`, `GET /stats/history` |
| `admin` | `/secrets/*`, `/users/*`, `GET /audit-logs`, `POST /workflows/{name}/pause`, `POST /workflows/{name}/resume` |

Routes that were admin-only stay so: `POST /runs`, cancel, retry, replay, pause and resume
of a run, and every `admin` route also need the key's owner to be an admin, checked at
request time, so demoting the owner takes effect at once (`403 FORBIDDEN`). Approving,
rejecting and answering a gate keep their assignee and delegation rules on top of the
scope. On `GET /events`, Provider Account events reach a key only when it holds the
`admin` scope and its owner is an admin.

Signals and Provider Accounts have their own scopes (`signals_send`, `accounts_read`,
`accounts_manage`). Dashboard sessions (JWT) are not scope-checked: only the account's
admin flag applies to them.

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
IRONFLOW_ENV=development cargo run -p ironflow-example-server
```

The server starts on `http://localhost:3000`. The dashboard is available at the root URL.
In development it generates its secrets and logs the worker token
(`start workers with WORKER_TOKEN=...`). Without `IRONFLOW_ENV=development`, set
`JWT_SECRET` and `WORKER_TOKEN` (`openssl rand -hex 32`): no secret is built in.

## Pausing runs and workflows

An administrator can hold work without cancelling it. Every route below needs an
admin account (403 otherwise):

| Route | Effect |
|-------|--------|
| `POST /api/v1/runs/{id}/pause` | Moves a root run and its active sub-workflow runs to `Paused`. A running step is interrupted and executed again on resume. |
| `POST /api/v1/runs/{id}/resume` | Puts a paused root run and its sub-workflow runs back in the state they were paused from. |
| `POST /api/v1/workflows/{name}/pause` | Workers stop picking the queued runs of the workflow; new runs are still created. |
| `POST /api/v1/workflows/{name}/resume` | Workers pick the queued runs of the workflow again. |

An unknown run or workflow answers 404. Pausing a finished, already paused or
sub-workflow run, or resuming a run that is not paused, answers 400. The CLI
offers the same actions as `ironflow-cli run pause|resume <id>` and
`ironflow-cli workflow pause|resume <name>`. See
[Pausing runs and workflows](../concepts/engine-worker.md#pausing-runs-and-workflows)
for the lifecycle details.
