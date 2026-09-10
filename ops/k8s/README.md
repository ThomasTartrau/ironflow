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

## Authentication

Register `kubeconfig` in your workflow's secret store. If not provided, falls back to in-cluster configuration:

```yaml
secrets:
  - name: kubeconfig
    env: KUBECONFIG_CONTENT   # full kubeconfig YAML content
```
