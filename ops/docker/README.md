# ironflow-ops-docker

Docker integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, powered by the [`bollard`](https://crates.io/crates/bollard) crate.

Provides Docker operations (containers, images, networks, volumes, system) as tracked workflow steps.

## Usage

```toml
[dependencies]
ironflow-ops-docker = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_docker::DockerClient;

// Connect to the local Docker daemon (Unix socket or Windows named pipe)
let client = DockerClient::connect_local()?;

// From a workflow step
let client = DockerClient::from_context(&ctx)?;

// Remote Docker daemon via URL
let client = DockerClient::connect_with_url("tcp://192.168.1.100:2376", 120)?;
```

### Tracked operations

```rust,ignore
use ironflow_ops_docker::DockerClient;
use ironflow_ops_docker::containers::ContainerCreate;

let docker = DockerClient::from_context(&ctx)?;

let create = ContainerCreate::new(&docker, "my-nginx", "nginx:latest");

let output = ctx.operation("create-nginx", &create).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `containers` | Create, start, stop, restart, remove, exec, inspect, list, logs, cleanup |
| `images` | Pull, push, build, remove, inspect, list, tag |
| `networks` | Create, remove, inspect, list, connect, disconnect |
| `volumes` | Create, remove, inspect, list |
| `system` | Info, version, disk usage, prune |

## Authentication

No secrets required for local Docker socket connections. For remote TCP connections, use `DockerClient::connect_with_url`.
