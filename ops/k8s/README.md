# ironflow-ops-k8s

Kubernetes integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, powered by the [`kube`](https://crates.io/crates/kube) crate.

Provides typed access to every Kubernetes resource and verb as tracked workflow steps. Re-exports the full `kube` and `k8s_openapi` APIs.

## Usage

```toml
[dependencies]
ironflow-ops-k8s = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_k8s::KubeClient;

// From a workflow step (reads kubeconfig from the secret store, or falls back to in-cluster config)
let kube = KubeClient::from_context(&ctx).await?;

// Explicitly from in-cluster configuration
let kube = KubeClient::new_in_cluster().await?;
```

### Tracked operations

```rust,ignore,ignore
use ironflow_ops_k8s::{KubeClient, KubeOp, verb};
use k8s_openapi::api::apps::v1::Deployment;

let kube = KubeClient::from_context(&ctx).await?;

let deployments = kube.namespaced::<Deployment>("production");
let op = kube.op(deployments, verb::Get::new("my-app"));

let output = ctx.operation("get-deployment", &op).await?;
```

### Direct kube API access

```rust,ignore,ignore
use ironflow_ops_k8s::KubeClient;
use ironflow_ops_k8s::kube::api::{Api, ListParams};
use k8s_openapi::api::core::v1::Pod;

let kube = KubeClient::from_context(&ctx).await?;
let pods: Api<Pod> = Api::namespaced(kube.client().clone(), "default");
let list = pods.list(&ListParams::default()).await?;
```

## Available verbs

Verb structs in the `verb` module: `List`, `Get`, `Create`, `Update`, `Patch`, `Delete`, `DeleteCollection`.

## Run-to-completion operations

Higher-level operations that manage a pod or an object as a single tracked step:

| Operation | Description |
|-----------|-------------|
| `PodRun` | Create an ephemeral pod, run a `sh -c` command, wait for completion, collect logs, delete the pod |
| `PodUp` | Start the pod of a `PodRun` as a service, wait until it is `Ready`, return its IP and leave it running |
| `JobRun` | Same via a `batch/v1` Job (with `backoffLimit` retries) |
| `ApplyConfigMap` | Server-side apply a `ConfigMap` from a key/value map |
| `ApplySecret` | Server-side apply a `Secret` from a key/value map (`input()` logs keys only) |

`PodRun`/`JobRun` treat a command that exits non-zero as `Ok { success: false }`,
not an error; an `OperationError::External { origin: "kubernetes", .. }` is
returned only for infrastructure failures (create, wait, timeout). The pod/Job
is always deleted, including on timeout.

`PodRunOutput` also carries `exit_code` (the container's `terminated.exitCode`)
and `reason`: the pod's own reason when Kubernetes set one (`DeadlineExceeded`
for a pod killed by `active_deadline_seconds`, `Evicted`), else the
container's (`Completed`, `Error`, `OOMKilled`). A pod killed by its deadline
has a `reason` and no `exit_code`. `logs` interleaves stdout and stderr, as the
pod log API serves them: redirect stderr in the command (`2>/dev/null`) to
parse stdout alone.

`PodRun::stdin(input)` feeds `input` to the command's stdin, up to 255 KiB
(`MAX_STDIN_BYTES`). It is stored in the `ironflow.io/stdin` annotation and
projected into a read-only file by a downward API volume: no extra RBAC, no
object to clean up, but readable by whoever can read the pod, so keep secrets
in a Secret.

`PodRun::sandboxed()` hardens the pod for a command you do not trust:
read-only root filesystem, all capabilities dropped, seccomp `RuntimeDefault`,
no privilege escalation, non-root (uid `10001` unless `.security(..)` sets
one), `/tmp` on a 512Mi `emptyDir` (`.tmp_size_limit("2Gi")`), no service
account token.

```rust,ignore
let out = ctx
    .operation(
        "extract",
        &PodRun::new(&kube, "extract", "python:3.13-slim", "python /scripts/extract.py")
            .sandboxed()
            .stdin(document_json)
            .active_deadline_seconds(Duration::from_secs(60)),
    )
    .await?;
match (out.exit_code, out.reason.as_deref()) {
    (_, Some("DeadlineExceeded")) => { /* timed out */ }
    (Some(0), _) => { /* out.logs is the script's output */ }
    (code, reason) => { /* failed: code, reason */ }
}
```

`PodRun::runtime_class("gvisor")` sets `spec.runtimeClassName` to run the pod
under a sandboxed runtime; left unset, the cluster default applies.

```rust,ignore
use ironflow_ops_k8s::{KubeClient, pod_run::PodRun};

let kube = KubeClient::from_context(&ctx).await?;
let run = PodRun::new(&kube, "run-tests", "rust:1.94", "cargo test")
    .namespace("ci")
    .pvc("workspace-claim", "/workspace")   // first PVC -> volume "workspace"
    .pvc("cargo-cache", "/cache")           // additive: second PVC -> volume "workspace-1"
    .working_dir("/workspace");

let output = ctx.operation("run-tests", &run).await?;
```

`.pvc()` is additive on both `PodRun` and `JobRun`: call it once per volume to
mount several PVCs in the same pod (e.g. an RWX workspace plus a node-local
cache). A single call reproduces the historical single-volume manifest exactly.

### Service pods

`PodUp` starts a pod that must outlive its step (a headless browser, a
database, the app under test). It takes a configured `PodRun` and a lifetime,
creates the pod with an optional readiness probe, and returns
`PodUpOutput { pod, namespace, ip }` as soon as the pod is `Ready`. Without a
probe, the pod is ready once its container runs.

```rust,ignore
use ironflow_ops_k8s::{PodRun, PodUp, PodUpOutput};
use k8s_openapi::api::core::v1::{ExecAction, Probe};

let db = ctx
    .operation(
        "db-up",
        &PodUp::new(
            PodRun::new(&kube, "db", "postgres:17", "docker-entrypoint.sh postgres")
                .label(LABEL_RUN_ID, &ctx.run_id().to_string()),
            Duration::from_secs(1800), // lifetime: steps that use it included
        )
        .readiness_probe(Probe {
            exec: Some(ExecAction { command: Some(vec!["pg_isready".into()]) }),
            period_seconds: Some(2),
            ..Default::default()
        })
        .ready_timeout(Duration::from_secs(120)),
    )
    .await?
    .json::<PodUpOutput>()?;
// db.ip is reachable from the next steps.
```

The lifetime replaces the `PodRun` timeout: `ironflow.io/expires-at` is
creation + lifetime + expiry margin, so the reaper leaves the service alone
while the run needs it and deletes it afterwards. A pod that ends, or is not
ready within `ready_timeout` (300s by default), fails the step with the reason
(phase, reason and exit code, or why its container is waiting, such as
`ImagePullBackOff`) and the last 40 lines of its log; that pod is deleted.

`release_run` and the reaper run in the namespace of the Kubernetes agent
provider only. A `PodUp` pod elsewhere, or under another provider, stays until
a reaper of its namespace passes its `expires-at`, and a new attempt of the
run cannot create a pod of the same name meanwhile: put the run id in its name.

With the Kubernetes agent provider, every new execution of the run (a resume
after an approval, a signal, a human input or a crashed worker) starts with
`release_run`, which deletes the pods tagged with the run, while the `PodUp`
step is replayed with the IP of the deleted pod. Keep the steps that use the
service in the same execution as the `PodUp`, with no such gate in between.

### Orphans and retries

A `PodRun` pod and a `JobRun` Job carry the conventions of the ironflow agent
pods (module `conventions`): `app.kubernetes.io/managed-by=ironflow`,
`app.kubernetes.io/component=pod-run` / `job-run`, and `ironflow.io/expires-at`
= creation + timeout + `.expiry_margin(d)` (60s by default). If the worker dies,
`reap_orphans` of `ironflow-core` (feature `transport-k8s`) deletes them once
expired. `.label()` refuses `managed-by` and `component`.

Tag the object with the run so that a retry of the run deletes whatever the
dead attempt left running, before the first step:

```rust,ignore
use ironflow_core::provider::{LABEL_ROOT_RUN_ID, LABEL_RUN_ID};

let run = PodRun::new(&kube, "check", "rust:1.94", "cargo test")
    .label(LABEL_RUN_ID, &ctx.run_id().to_string())
    // Inside a sub-workflow: the top-level run is the one that is retried.
    .label(LABEL_ROOT_RUN_ID, &ctx.root_run_id().to_string());
```

## Authentication

Register `kubeconfig` in your workflow's secret store. If not provided, falls back to in-cluster configuration:

```yaml
secrets:
  - name: kubeconfig
    env: KUBECONFIG_CONTENT   # full kubeconfig YAML content
```
