# Kubernetes Sandbox

`K8sEphemeralProvider::sandboxed(image)` runs every agent step in a hardened,
single-use pod. Everything is opt-in: `K8sEphemeralProvider::new(image)` keeps
its behaviour, apart from the run/step labels, the expiry annotation and the
cleanup of a previous attempt described below.

## The image

The official image `ironflow-claude-runner:<claude-code-version>-<n>` is built
from `docker/claude-runner/` by the `build-claude-runner-image` CI job. There is
no `latest` tag: pin the full tag.

| Path | Content |
|------|---------|
| `/home/claude` | `HOME` of uid `10001`, an `emptyDir` in the sandbox |
| `/tmp` | `TMPDIR`, an `emptyDir` in the sandbox |
| `/etc/claude-code` | `managed-settings.json` (baked default, replaced by a preset) |
| `/etc/ironflow/claude-profile` | Claude profile ConfigMap, copied into `~/.claude` |

```dockerfile
{{#include ../../../../docker/claude-runner/Dockerfile}}
```

## `sandboxed()` defaults

| Default | Relaxation |
|---------|------------|
| Runs as uid/gid `10001`, `runAsNonRoot`, seccomp `RuntimeDefault` | `.run_as_user(uid)` (never `0`) |
| Read-only root filesystem, all capabilities dropped, no privilege escalation | `.allow_writable_root()` |
| `HOME` on a 1Gi `emptyDir` | `.home_size_limit("4Gi")` |
| `/tmp` on a 512Mi `emptyDir` | `.tmp_size_limit("2Gi")` |
| `activeDeadlineSeconds` = timeout + 60s | `.deadline_margin(d)`, or `.active_deadline_seconds(d)` to set it outright |
| No service account token mounted | `.service_account(name)` on the provider or the step |
| Secrets refused as plain text | none: use a Secret |

A relaxation called on a provider built with `new()` panics.

## Secrets

A sandboxed provider refuses `oauth_credentials(json)` and
`env("ANTHROPIC_API_KEY", ..)` / `env("CLAUDE_CODE_OAUTH_TOKEN", ..)`: the
value would sit in clear text in the pod spec. Read it from a Kubernetes Secret
instead; the pod only carries a `secretKeyRef`:

```rust,ignore
let provider = K8sEphemeralProvider::sandboxed(&image)
    // Long-lived token from `claude setup-token`.
    .oauth_token_from_secret("claude-oauth", "token")
    // Or the full credentials JSON, written to ~/.claude/.credentials.json.
    .oauth_credentials_from_secret("claude-credentials", "credentials.json")
    // Any other variable.
    .env_from_secret("GITLAB_TOKEN", "gitlab-bot", "token");
```

## Per-step settings

A step adds to or overrides the provider's settings through its
`AgentConfig`. Other providers ignore these fields.

```rust,ignore
let config = AgentConfig::new("Open the merge request")
    .env_from_secret("GITLAB_TOKEN", "gitlab-bot", "token") // wins over the provider's
    .service_account("gitlab-reader")                       // wins over the provider's
    .read_only_pvc("repos", "/data/repos")                  // after the provider's volumes
    .read_only_config_map("guidelines", "/data/guidelines")
    .managed_settings("readonly")                           // preset registered on the provider
    .egress_profile("gitlab");                              // ironflow.io/egress-profile label
```

Read-only mounts must be absolute, unique, and cannot shadow `/`,
`/home/claude`, `/tmp`, `/etc/claude-code` or `/etc/ironflow/claude-profile`.

Managed-settings presets map a name to a ConfigMap holding
`managed-settings.json`. An unknown preset fails the step, never falls back:

```rust,ignore
let provider = K8sEphemeralProvider::sandboxed(&image)
    .managed_settings_preset("locked", "claude-managed-locked")
    .managed_settings_preset("readonly", "claude-managed-readonly")
    .default_managed_settings("locked");
```

## Labels and retry cleanup

The engine stamps `ironflow.io/run-id` and `ironflow.io/step` on every agent
step (outside the engine, call `AgentConfig::run_scope(run_id, step)`). Step
names are sanitized into valid label values, with a hash suffix when altered.

Before creating a pod, the provider deletes the pods and prompt ConfigMaps of a
previous attempt of the same step of the same run, and waits until the pods
are gone (`.previous_attempt_timeout(d)`, 60s by default). If they are still
terminating, the step fails: two agents never run side by side. Step names must
therefore be unique within a parallel group.

## The reaper

Every pod and prompt ConfigMap carries the `ironflow.io/expires-at` annotation
(unix seconds: creation + timeout + margin). The reaper deletes pods past that
time, pods Kubernetes killed with `DeadlineExceeded`, and expired prompt
ConfigMaps. Objects without a parseable annotation are never touched.

```rust,ignore
let report = provider.reap_orphans().await?;            // one pass
let handle = provider.spawn_orphan_reaper(Duration::from_secs(300)); // background
```

## Network policies

The worker does not create network policies: its Role in
`examples/k8s/sandbox/namespace-rbac.yaml` has no access to them. A cluster
administrator applies a default deny (`networkpolicy-deny-all.yaml`) and FQDN
openings selected on the `ironflow.io/egress-profile` label:

```yaml
{{#include ../../../../examples/k8s/sandbox/cilium-egress-anthropic.yaml}}
```

## Full example

```rust,ignore
{{#include ../../../../examples/transports/src/bin/k8s_sandboxed.rs}}
```

## Checks

Run these against a live agent pod before trusting the setup:

```sh
# Non-root, read-only root filesystem, capabilities dropped.
kubectl -n ironflow-agents get pod -l app.kubernetes.io/component=claude-runner \
  -o jsonpath='{.items[0].spec.containers[0].securityContext}'
# No secret value in the spec, only secretKeyRef.
kubectl -n ironflow-agents get pod <pod> -o yaml | grep -A3 CLAUDE_CODE_OAUTH_TOKEN
# No service account token mounted.
kubectl -n ironflow-agents exec <pod> -- ls /var/run/secrets/kubernetes.io/serviceaccount
# Root filesystem is read-only.
kubectl -n ironflow-agents exec <pod> -- touch /usr/local/probe
# Egress is limited to the profile.
kubectl -n ironflow-agents exec <pod> -- curl -sS -m 5 https://example.com
# The worker cannot touch network policies.
kubectl auth can-i create networkpolicies -n ironflow-agents \
  --as=system:serviceaccount:ironflow:ironflow-worker
# Run and step labels, and the expiry annotation.
kubectl -n ironflow-agents get pods -L ironflow.io/run-id,ironflow.io/step \
  -o custom-columns='NAME:.metadata.name,EXPIRES:.metadata.annotations.ironflow\.io/expires-at'
```

The `exec` checks against the service account, the root filesystem and egress
must fail; `kubectl auth can-i` must answer `no`.
