# Kubernetes Sandbox

`K8sEphemeralProvider::sandboxed(image)` runs every agent step in a hardened,
single-use pod. Everything is opt-in: `K8sEphemeralProvider::new(image)` keeps
its behaviour, apart from the run/step labels, the expiry annotation and the
cleanup of a previous attempt described below.

## The image

The official image
`registry.gitlab.com/thomastartrau/ironflow/ironflow-claude-runner:<claude-code-version>-<n>`
is built from `docker/claude-runner/` by the `build-claude-runner-image` CI job.
The current tag is in `docker/claude-runner/IMAGE_TAG`. There is no `latest`
tag: pin the full tag.

| Path | Content |
|------|---------|
| `/home/claude` | `HOME` of uid `10001`, an `emptyDir` in the sandbox |
| `/tmp` | `TMPDIR`, an `emptyDir` in the sandbox |
| `/etc/claude-code` | `managed-settings.json` (baked default, replaced by a preset) |
| `/etc/ironflow/claude-profile/<n>` | Claude profile ConfigMaps, copied into `~/.claude/<subdir>` |

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

## Auth proxy: no Claude credential in the pod

A `secretKeyRef` keeps the credential out of the pod spec, not out of the pod:
anything running in the agent container can still read
`CLAUDE_CODE_OAUTH_TOKEN`. With `auth_proxy`, the pod never receives it:

```rust,ignore
let provider = K8sEphemeralProvider::sandboxed(&image)
    .namespace("ironflow-agents")
    .auth_proxy("http://ironflow-auth-proxy.ironflow-system");
```

The `ironflow-auth-proxy` service (crate `ironflow-auth-proxy`, manifests in
`examples/k8s/sandbox/auth-proxy.yaml`) holds the credential instead. Its
official image
`registry.gitlab.com/thomastartrau/ironflow/ironflow-auth-proxy:<version>` is
built from `docker/auth-proxy/Dockerfile` by the `build-auth-proxy-image` CI
job. `<version>` is the version of the `ironflow-auth-proxy` crate: the job
publishes it once that version is released, and never rebuilds a published
tag. There is no `latest` tag: pin the version.

- **Token lifecycle.** At pod launch the worker calls the proxy admin API and
  gets an opaque token (`ifap_...`) bound to the run id, the step and the pod
  expiry (`ironflow.io/expires-at`). The pod receives `ANTHROPIC_BASE_URL` (the
  proxy URL), `ANTHROPIC_AUTH_TOKEN` (the opaque token) and
  `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`. The worker revokes the token
  when the step ends, whatever the outcome, and every token of a run when the
  run is released; the expiry is the backstop. An unknown, revoked or expired
  token gets a 401. If the proxy cannot issue a token, the step fails before
  any pod is created.
- **Restrictions.** The proxy only relays GET and POST requests under `/v1/`
  to `api.anthropic.com`. A path outside `/v1/` (including `/admin`, which needs
  the admin key) or a request for another host (absolute-form URI, `CONNECT`) gets
  a 403, any other method a 405.
- **Which credential.** The step's Provider Account when one is attached (the
  provider reports the `claude_subscription` account kind once `auth_proxy` is
  set), else `CLAUDE_CODE_OAUTH_TOKEN`, then `ANTHROPIC_API_KEY`, from the worker
  environment. The worker also needs `IRONFLOW_AUTH_PROXY_ADMIN_KEY` (or
  `.auth_proxy_admin_key(..)`). Rate-limit windows reported through the proxy
  are recorded on the Provider Account as with the Docker provider.
- **No credential on the pod side.** With `auth_proxy` set, a Claude credential
  configured for the pod (`oauth_token_from_secret`, `oauth_credentials`,
  `oauth_credentials_from_secret`, a step `env_from_secret("CLAUDE_CODE_OAUTH_TOKEN", ..)`,
  any plain value starting with `sk-ant`) fails the step with an error naming
  the variable.
- **Registry.** In memory by default (one replica). Set
  `IRONFLOW_AUTH_PROXY_DATABASE_URL` and an encryption key
  (`IRONFLOW_SECRET_KEYS`) for a shared PostgreSQL registry: several replicas,
  tokens survive restarts, the proxy needs egress to PostgreSQL.
- **Logs.** The proxy and the worker log the first 12 characters of the token
  id (a SHA-256 of the token), never the token or the credential.

The network side changes too: the agent pods only reach the proxy, and the
proxy only reaches `api.anthropic.com`:

```yaml
{{#include ../../../../examples/k8s/sandbox/cilium-egress-auth-proxy.yaml}}
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
    .runtime_class("gvisor")                                // wins over the provider's
    .egress_profile("gitlab");                              // ironflow.io/egress-profile label
```

Read-only mounts must be absolute, unique, and cannot shadow `/`,
`/home/claude`, `/tmp`, `/etc/claude-code`, or `/etc/ironflow/claude-profile`
and anything below it.

## Claude profile

A Claude profile (`CLAUDE.md`, `settings.json`, `rules/`, `agents/`,
`commands/`) reaches `~/.claude` through ConfigMaps. A ConfigMap key cannot
contain `/`, so each directory of the profile is its own ConfigMap, mapped to
its sub-directory:

```rust,ignore
let provider = K8sEphemeralProvider::sandboxed(&image)
    .claude_profile_configmap("claude-profile")                 // ~/.claude
    .claude_profile_configmap_at("claude-profile-rules", "rules") // ~/.claude/rules
    .claude_profile_configmap_at("claude-profile-agents", "agents");
```

Each ConfigMap is mounted read-only in its own directory, then only its keys
are copied into `~/.claude/<subdir>` before the agent starts: never the
`..data` entries of the volume. Profiles are copied before the credentials,
which they cannot overwrite. A `subdir` must be relative, made of
`[A-Za-z0-9._-]` segments without `.` or `..`, and mapped once: the provider
panics at build time otherwise.

With kustomize, one `configMapGenerator` per directory. Disable the name hash:
the provider refers to the ConfigMaps by name.

```yaml
# kustomization.yaml, next to claude-home/
namespace: ironflow-agents
generatorOptions:
  disableNameSuffixHash: true
configMapGenerator:
  - name: claude-profile
    files:
      - claude-home/CLAUDE.md
      - claude-home/settings.json
  - name: claude-profile-rules
    files:
      - claude-home/rules/rust.md
      - claude-home/rules/security.md
```

`kustomize` lists every file. To pick up a whole directory instead:
`kubectl create configmap claude-profile-rules --from-file=claude-home/rules/
--dry-run=client -o yaml`.

Managed-settings presets map a name to a ConfigMap holding
`managed-settings.json`. An unknown preset fails the step, never falls back:

```rust,ignore
let provider = K8sEphemeralProvider::sandboxed(&image)
    .managed_settings_preset("locked", "claude-managed-locked")
    .managed_settings_preset("readonly", "claude-managed-readonly")
    .default_managed_settings("locked");
```

## Labels and retry cleanup

The engine stamps `ironflow.io/run-id`, `ironflow.io/root-run-id` and
`ironflow.io/step` on every agent step (outside the engine, call
`AgentConfig::run_scope(run_id, step)`). Step names are sanitized into valid
label values, with a hash suffix when altered. The root run is the run itself,
or the top-level run inside a sub-workflow.

Before every execution of a run, first one included, the engine calls
`release_run`: the provider deletes every pod, `JobRun` Job and prompt
ConfigMap labelled with the run id or with the run as root, and waits until
the pods are gone (`.previous_attempt_timeout(d)`, 60s by default). A retry
that starts by resetting shared state (a worktree) never runs next to an
agent of the dead attempt still writing to it. If the pods are still there, or
the Kubernetes API fails, the execution fails with a replayable error and the
next attempt tries again.

Tag a pod you create yourself with the same labels so that it is released too:

```rust,ignore
let run = PodRun::new(&kube, "check", &image, "cargo test")
    .label(LABEL_RUN_ID, &ctx.run_id().to_string())
    .label(LABEL_ROOT_RUN_ID, &ctx.root_run_id().to_string());
```

Before creating a pod, the provider also deletes the pods and prompt
ConfigMaps of a previous attempt of the same step of the same run. If they are
still terminating, the step fails: two agents never run side by side. Two
branches of a `ctx.parallel()` group with the same name would delete each
other's pod, so the engine fails such a group before creating any step.

Deleting `JobRun` Jobs needs `list` and `delete` on `jobs`; without them, Jobs
are skipped with a warning and the pods are still released.

## The reaper

Every object ironflow creates carries `app.kubernetes.io/managed-by=ironflow`,
an `app.kubernetes.io/component` (`claude-runner`, `prompt-data`, `pod-run`,
`job-run`) and the `ironflow.io/expires-at` annotation (unix seconds: creation
+ timeout + margin). That covers the agent pods and their prompt ConfigMaps,
and the pods of `PodRun` and Jobs of `JobRun` from `ironflow-ops-k8s`
(`.expiry_margin(d)`, 60s by default). A caller cannot set `managed-by` or
`component`: `pod_label`, `PodRun::label` and `JobRun::label` panic, an agent
step carrying one fails.

The reaper selects on `managed-by=ironflow` and deletes pods and Jobs past
their expiry or killed with `DeadlineExceeded`, and expired prompt ConfigMaps.
A Job goes with its pods; a pod a Job controls is left to its Job. Objects
without a parseable annotation are never touched.

```rust,ignore
let report = provider.reap_orphans().await?;            // one pass
let handle = provider.spawn_orphan_reaper(Duration::from_secs(300)); // background
// Without a K8sEphemeralProvider, e.g. a worker that only runs PodRun:
let report = reap_orphans(&K8sClusterConfig::Default, "ironflow-agents").await?;
```

Reaping Jobs needs `list` and `delete` on `jobs` (see
`examples/k8s/sandbox/namespace-rbac.yaml`). Without them, the Job pass is
skipped with a warning and the rest of the pass runs.

## gVisor (RuntimeClass)

A `runc` container shares the node kernel: a kernel exploit in untrusted code
reaches the node. gVisor runs the pod against a user-space kernel instead. On
Talos, install the `gvisor` system extension on the nodes, then declare the
RuntimeClass once per cluster:

```yaml
apiVersion: node.k8s.io/v1
kind: RuntimeClass
metadata:
  name: gvisor
handler: runsc
```

Set it on the provider to cover every agent pod, and override it for one step
(the step wins):

```rust,ignore
let provider = K8sEphemeralProvider::sandboxed(&image)
    .runtime_class("gvisor");

let config = AgentConfig::new("Review this untrusted patch")
    .runtime_class("kata"); // wins over the provider's
```

A check pod created with `PodRun` takes the same setting:

```rust,ignore
let run = PodRun::new(&kube, "check", "rust:1.94", "cargo test")
    .runtime_class("gvisor");
```

Without a value, `spec.runtimeClassName` stays absent and the cluster default
runtime (`runc`) applies. A blank name is refused.

> **Warning:** builds are noticeably slower under gVisor (compilation, many
> small file syscalls, `cargo` and `npm` installs). Keep gVisor for the steps
> that handle untrusted code and leave trusted build steps on `runc`.

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
# Run, root run and step labels, and the expiry annotation.
kubectl -n ironflow-agents get pods \
  -L ironflow.io/run-id,ironflow.io/root-run-id,ironflow.io/step \
  -o custom-columns='NAME:.metadata.name,EXPIRES:.metadata.annotations.ironflow\.io/expires-at'
```

The `exec` checks against the service account, the root filesystem and egress
must fail; `kubectl auth can-i` must answer `no`.

With the auth proxy, also check that the pod holds no Claude secret and that
the proxy refuses an unknown token:

```sh
kubectl -n ironflow-agents exec <pod> -- env | grep -c 'sk-ant'   # 0
kubectl -n ironflow-agents exec <pod> -- sh -c \
  'curl -s -o /dev/null -w "%{http_code}" -H "Authorization: Bearer invalide" "$ANTHROPIC_BASE_URL/v1/messages"'   # 401
```
