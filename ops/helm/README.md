# ironflow-ops-helm

Helm integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, wrapping the Helm CLI.

Provides Helm operations (install, upgrade, rollback, chart management, repository, registry) as tracked workflow steps. Each operation spawns the `helm` binary and parses the output.

## Usage

```toml
[dependencies]
ironflow-ops-helm = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_helm::HelmClient;

// From a workflow step (reads helm_binary, helm_kubeconfig, helm_namespace from the secret store)
let helm = HelmClient::from_context(&ctx).await?;

// Explicit configuration
let helm = HelmClient::new("helm", Some("/path/to/kubeconfig"), Some("production"));
```

### Tracked operations

```rust,ignore
use ironflow_ops_helm::HelmClient;
use ironflow_ops_helm::release::Install;

let helm = HelmClient::from_context(&ctx).await?;

let install = Install::new(helm, "my-release", "bitnami/nginx")
    .namespace("web")
    .values_file("values.yaml");

let output = ctx.operation("install-nginx", &install).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `chart` | Dependency update, lint, package, pull, push, show, template, verify |
| `plugin` | Install, uninstall, list, update |
| `registry` | Login, logout |
| `release` | Install, upgrade, rollback, uninstall, get, history, list, status, test |
| `repo` | Add, remove, update, list, index |
| `util` | Version |

## Authentication

All secrets are optional. Register in your workflow's secret store as needed:

```yaml
secrets:
  - name: helm_binary       # path to the helm binary (defaults to "helm")
    env: HELM_BINARY
  - name: helm_kubeconfig   # path to the kubeconfig file
    env: HELM_KUBECONFIG
  - name: helm_namespace    # default namespace
    env: HELM_NAMESPACE
```
