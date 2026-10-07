# ironflow-auth-proxy

Auth proxy for ironflow agent pods. With `K8sEphemeralProvider::auth_proxy`,
the agent pod never receives the Claude credential: the worker issues an
opaque token bound to one run and one step, the pod sends it to this proxy as
`Authorization: Bearer`, and the proxy swaps it for the real credential before
relaying the request to `https://api.anthropic.com`.

- Anthropic relay: GET and POST under `/v1/` only. An unknown, expired or revoked token
  gets a 401, a path outside `/v1/` or a request for another host a 403, any
  other method a 405. Responses (SSE included) are streamed, rate-limit headers
  kept.
- Admin API (`/admin/v1/tokens`, `/admin/v1/tokens/{id}`,
  `/admin/v1/runs/{run_id}/tokens`): protected by the admin key.
- Proxied secrets: `/r/<host>/<path>` relays to `https://<host>/<path>` with
  a GitHub, GitLab or other secret the pod never sees (see below).
- Metrics: `GET /metrics` (Prometheus text format, same port).
- Logs (JSON) never contain a token, a credential, a secret, a header or a
  query string, only the first 12 characters of the token id.

## Proxied secrets

A grant can hold a secret instead of the Claude credential
(`K8sEphemeralProvider::proxied_secret` / `AgentConfig::proxied_secret`). The
pod gets an opaque token in `<env>` and the relay base `<proxy>/r` in
`<env>_URL`, and calls `<env>_URL/<host>/<path>` presenting the token as
`Authorization: Bearer`, `x-api-key`, `Private-Token` or the password of
`Authorization: Basic`. The proxy checks the host against the grant's
allowlist, strips the token and injects the real secret:

| Injection | Header sent upstream |
|-----------|----------------------|
| `bearer` | `Authorization: Bearer <secret>` |
| `private_token` | `Private-Token: <secret>` (GitLab) |
| `x_api_key` | `x-api-key: <secret>` |
| `header` | `<name>: <secret>` |
| `basic` | `Authorization: Basic base64(<username>:<secret>)` (git over https) |

- Allowlist: exact host names (`api.github.com`) or a leading wildcard
  (`*.example.com`, which matches `api.example.com` but not `example.com`). No
  regex, no port, no IP address, https only. A host outside it gets a 403
  `forbidden_host`, so does a Claude token on `/r/` and a secret token on the
  Anthropic API.
- Path: `..`, `//` and percent-encoding are refused (403 `forbidden_path`);
  the query string is relayed untouched. Methods: GET, HEAD, POST, PUT, PATCH
  and DELETE (405 `forbidden_method` otherwise).
- Redirects are returned to the pod with their `Location`, never followed: a
  redirect never carries the secret to another host.
- Unknown, expired and revoked tokens get a 401. A revoked grant is dropped
  with its secret at once; its id is kept until its expiry so logs tell
  `revoked` from `unknown_token`.
- Each `/r/` request logs one `secret relay` event with `host`, `secret` (the
  name), `run_id`, `step`, `token` (short id), `result` and
  `upstream_status`, and increments
  `ironflow_auth_proxy_requests_total{secret, result}`. Results: `relayed`,
  `forbidden_host`, `forbidden_path`, `forbidden_method`, `unknown_token`,
  `expired`, `revoked`, `upstream_error`, `unavailable`.
- The proxy pods need egress to every allowlisted host: see the commented
  `toFQDNs` rule of `examples/k8s/sandbox/cilium-egress-auth-proxy.yaml`.

## Environment

| Variable | Default | Description |
|----------|---------|-------------|
| `IRONFLOW_AUTH_PROXY_ADMIN_KEY` | - (required) | Admin key, at least 32 characters, shared with the worker. |
| `IRONFLOW_AUTH_PROXY_LISTEN` | `0.0.0.0:8080` | Listen address. |
| `IRONFLOW_AUTH_PROXY_DATABASE_URL` | unset (in memory) | PostgreSQL URL of the shared token registry. Never logged. |
| `IRONFLOW_SECRET_KEYS` | - (required with a database) | Key ring encrypting the credentials at rest, `version:hex` entries (64 hex characters each), comma-separated. |
| `IRONFLOW_SECRET_ACTIVE_KEY_VERSION` | highest version | Key version new grants are encrypted with. |
| `IRONFLOW_SECRET_KEY` | - | Legacy single key (version 1), used when `IRONFLOW_SECRET_KEYS` is unset. |
| `RUST_LOG` | `info` | Log filter. |

## Registry backends

**In memory (default).** Run exactly one replica with the `Recreate`
strategy: a second one would refuse the tokens issued by the first, and a
restart drops the tokens of the steps in flight.

**PostgreSQL**, when `IRONFLOW_AUTH_PROXY_DATABASE_URL` is set. Several
replicas share the registry, with the `RollingUpdate` strategy, and tokens
survive restarts.

- Stored per token: its SHA-256 (never the token itself), the run, the step,
  the expiry and the credential, AES-256-GCM encrypted with the key ring.
  For a proxied secret, its name, injection and allowlist are stored in
  clear next to it; only the value is encrypted.
  Startup fails when the database is set without an encryption key.
- Expired rows are purged every 60 s by every replica (idempotent deletes).
- A database outage answers 503 (retryable), never 401: a valid token does not
  look revoked.
- The network policy must allow egress from the proxy to PostgreSQL: under
  Cilium, uncomment the 5432 rule of
  `examples/k8s/sandbox/cilium-egress-auth-proxy.yaml`.
- Least privilege: give the proxy a dedicated database or role. Its migrations
  create the `ironflow` schema in that database.

## Deployment

Image:
`registry.gitlab.com/thomastartrau/ironflow/ironflow-auth-proxy:<version>`,
where `<version>` is the version of this crate. CI builds it from
`docker/auth-proxy/Dockerfile` and publishes it once the version is released;
a published tag is never rebuilt, and there is no `latest`. Manifests and
network policies:
`examples/k8s/sandbox/` (`auth-proxy.yaml`, `cilium-egress-auth-proxy.yaml`,
`networkpolicy-auth-proxy.yaml`).
