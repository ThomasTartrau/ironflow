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

For untrusted prompts or multi-tenant clusters, `K8sEphemeralProvider::sandboxed`
runs each agent in a hardened pod: non-root, read-only root filesystem, secrets
read from Kubernetes Secrets, managed-settings presets and egress profiles. See
[Kubernetes Sandbox](k8s-sandbox.md).

## Provider Account credentials

The local, Docker and SSH transports inject the credential of the
[Provider Account](../concepts/provider-accounts.md) the worker selected:
through the process environment, the Docker exec environment, or the first line
of stdin over SSH. The token never appears on a command line. The Kubernetes
transports do not inject accounts yet and keep using the pod environment.

## Resuming an interrupted agent step

Before an agent step launches, the engine pins the Claude Code session it runs
in (`--session-id <uuid>`) and records it on the step as `session_id`. When the
worker running the step loses its lease, the step is interrupted (see
[Engine & Worker](../concepts/engine-worker.md#resuming-after-a-lost-lease)).
The next execution of the same step, in the same attempt, resumes that session
(`--resume <uuid>`) with a resume prompt instead of starting the agent from
scratch, and keeps the conversation and the work already done.

The resume prompt defaults to `DEFAULT_RESUME_PROMPT`. Set your own on the step:

```rust,ignore
ctx.agent(
    "review",
    AgentStepConfig::new("Review the diff")
        .max_budget_usd(0.50)
        .resume_prompt("You were interrupted. Finish the review where you stopped."),
)
.await?;
```

Claude Code keeps its sessions under `~/.claude/projects/`, keyed by the
working directory. A step can only resume a session the transport can still
read:

| Transport | Session kept |
|-----------|--------------|
| Local | Yes, when the next worker runs on the same machine, under the same user and working directory |
| Docker | Yes, while the container lives |
| SSH | Yes, on the same host |
| Kubernetes persistent | Yes, while the pod lives |
| Kubernetes ephemeral | Only with `sessions_volume(claim)` on a sandboxed provider, see [Kubernetes Sandbox](k8s-sandbox.md) |

When the session is gone, the step does not fail: it logs
`session <uuid> not found, restarting the agent from scratch` and runs the
original prompt again in a session of the same id. A step that sets `resume`
or `session_id` itself is left alone.

Each resume writes the system log line
`agent step resumed from session <uuid>` on the step and publishes an
`agent_step_resumed` event on the run's event stream. Only a step interrupted
by a lost lease resumes: a step failed by the agent, a manual retry of the run
(a new attempt) or a parked step that never launched starts a fresh session.

Cost and tokens of a resumed step only cover the resume invocation. The
interrupted invocation never wrote its final `result` line, so what it spent
before the worker lost its lease stays unknown and is not added to the step,
the run or the budget counters. Expect the real spend of a resumed step to be
higher than the recorded one.

A step retry (`retry_policy`) stays in the session of the step: the retry sends
the original prompt into the session the first try created (the CLI refuses to
create a session id twice), so the step records a single session id.

## Choosing a transport

- **Development**: use `ClaudeCodeProvider` (local). No setup needed.
- **CI/CD pipelines**: Docker or Kubernetes for isolation.
- **Remote build servers**: SSH for machines you already manage.
- **Multi-tenant production**: Kubernetes ephemeral pods for strong isolation between tenants.
