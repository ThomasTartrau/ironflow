# ironflow-core

Core building blocks for the **ironflow** workflow engine. Provides composable, async operations that can be chained via plain Rust variables to build headless CI/CD, DevOps, and AI-powered workflows.

## Operations

| Operation | Description |
|-----------|-------------|
| `Shell` | Execute a shell command with timeout, env control, and `kill_on_drop` |
| `Agent` | Invoke an AI agent (Claude Code by default) with structured output support |
| `Http` | Perform HTTP requests via `reqwest` with builder-pattern ergonomics |

## Provider trait

The `AgentProvider` trait abstracts how agent commands are executed. Built-in providers:

| Provider | Feature flag | Description |
|----------|-------------|-------------|
| `ClaudeCodeProvider` | - | Local Claude Code CLI |
| `SshProvider` | `transport-ssh` | Remote execution via SSH |
| `DockerProvider` | `transport-docker` | Run inside a Docker container |
| `K8sEphemeralProvider` | `transport-k8s` | Ephemeral Kubernetes pods |
| `RecordReplayProvider` | - | Deterministic test fixtures without tokens |

## Features

| Feature | Description |
|---------|-------------|
| `prometheus` | Expose Shell, HTTP, and Agent metrics via `metrics` |
| `transport-ssh` | SSH-based remote agent execution |
| `transport-docker` | Docker-based remote agent execution |
| `transport-k8s` | Kubernetes-based ephemeral agent execution |

## Quick start

```rust,no_run
use ironflow_core::prelude::*;
use ironflow_core::operations::shell::Shell;
use ironflow_core::operations::agent::Agent;

async fn example() -> Result<(), OperationError> {
    // Shell command
    let output = Shell::new("echo hello").await?;
    println!("{}", output.stdout);

    // Agent with structured output
    let provider = ClaudeCodeProvider::default();
    let result = Agent::new("Summarize this text", provider).await?;
    println!("{}", result.text());
    Ok(())
}
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
