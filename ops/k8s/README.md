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

Higher-level operations that manage a full lifecycle as a single tracked step:

| Operation | Description |
|-----------|-------------|
| `PodRun` | Create an ephemeral pod, run a `sh -c` command, wait for completion, collect logs, delete the pod |
| `JobRun` | Same via a `batch/v1` Job (with `backoffLimit` retries) |
| `ApplyConfigMap` | Server-side apply a `ConfigMap` from a key/value map |
| `ApplySecret` | Server-side apply a `Secret` from a key/value map (`input()` logs keys only) |

`PodRun`/`JobRun` treat a command that exits non-zero as `Ok { success: false }`,
not an error; an `OperationError::External { origin: "kubernetes", .. }` is
returned only for infrastructure failures (create, wait, timeout). The pod/Job
is always deleted, including on timeout.

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

## Authentication

Register `kubeconfig` in your workflow's secret store. If not provided, falls back to in-cluster configuration:

```yaml
secrets:
  - name: kubeconfig
    env: KUBECONFIG_CONTENT   # full kubeconfig YAML content
```
