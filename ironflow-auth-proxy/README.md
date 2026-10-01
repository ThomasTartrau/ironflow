# ironflow-auth-proxy

Auth proxy for ironflow agent pods. With `K8sEphemeralProvider::auth_proxy`,
the agent pod never receives the Claude credential: the worker issues an
opaque token bound to one run and one step, the pod sends it to this proxy as
`Authorization: Bearer`, and the proxy swaps it for the real credential before
relaying the request to `https://api.anthropic.com`.

- Relay: GET and POST under `/v1/` only. An unknown, expired or revoked token
  gets a 401, a path outside `/v1/` or a request for another host a 403, any
  other method a 405. Responses (SSE included) are streamed, rate-limit headers
  kept.
- Admin API (`/admin/v1/tokens`, `/admin/v1/tokens/{id}`,
  `/admin/v1/runs/{run_id}/tokens`): protected by the admin key.
- Logs (JSON) never contain a token or a credential, only the first 12
  characters of the token id.

## Environment

| Variable | Default | Description |
|----------|---------|-------------|
| `IRONFLOW_AUTH_PROXY_ADMIN_KEY` | - (required) | Admin key, at least 32 characters, shared with the worker. |
| `IRONFLOW_AUTH_PROXY_LISTEN` | `0.0.0.0:8080` | Listen address. |
| `RUST_LOG` | `info` | Log filter. |

## Single replica

Tokens live in memory. Run exactly one replica: a second one would refuse the
tokens issued by the first, and a restart drops the tokens of the steps in
flight.

## Deployment

Image:
`registry.gitlab.com/thomastartrau/ironflow/ironflow-auth-proxy:<version>`,
where `<version>` is the version of this crate. CI builds it from
`docker/auth-proxy/Dockerfile` and publishes it once the version is released;
a published tag is never rebuilt, and there is no `latest`. Manifests and
network policies:
`examples/k8s/sandbox/` (`auth-proxy.yaml`, `cilium-egress-auth-proxy.yaml`,
`networkpolicy-auth-proxy.yaml`).
