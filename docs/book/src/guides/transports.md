# Transports

Transports control where agent steps execute. By default, agents run on the local machine via `ClaudeCodeProvider`. Ironflow ships additional transports for running agents in isolated environments.

## Available transports

| Transport | Provider | Use case |
|-----------|----------|----------|
| Local | `ClaudeCodeProvider` | Development, simple setups |
| Docker | `DockerProvider` | Isolated containers on the same host |
| SSH | `SshProvider` | Remote machines |
| Kubernetes | `K8sProvider` | Ephemeral or persistent pods in a cluster |

## Docker transport

Executes agent commands inside a running Docker container via `docker exec`:

```rust,ignore
{{#include ../../../../examples/transports/src/bin/docker_transport.rs}}
```

## SSH transport

Connects to a remote host via SSH:

```rust,ignore
{{#include ../../../../examples/transports/src/bin/ssh_transport.rs}}
```

## Kubernetes transport

Two modes are available:

- **Ephemeral** -- creates a pod for each agent call, deletes it when done
- **Persistent** -- reuses a long-lived pod for multiple calls

See the `examples/transports/` directory for complete Kubernetes examples.

## Choosing a transport

- **Development**: use `ClaudeCodeProvider` (local). No setup needed.
- **CI/CD pipelines**: Docker or Kubernetes for isolation.
- **Remote build servers**: SSH for machines you already manage.
- **Multi-tenant production**: Kubernetes ephemeral pods for strong isolation between tenants.
