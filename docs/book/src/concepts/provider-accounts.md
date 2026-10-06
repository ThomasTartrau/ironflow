# Provider Accounts

A **Provider Account** is an account at an AI provider: in v1, a Claude Pro/Max
subscription. Each account has a credential and usage limits (windows such as
the 5 hour and 7 day windows of a subscription). Admins manage accounts live
from the dashboard (**Settings > Accounts**), the REST API
(`/api/v1/provider-accounts`), the CLI (`ironflow accounts`) and the MCP server.

When at least one account of the right kind exists, the worker picks one for
every agent step, injects its credential into the Claude CLI process, and
records the usage windows the CLI reports. With no account, agent steps run
with the worker's own environment, exactly as before.

## The `claude_subscription` kind

Run `claude setup-token` on a machine logged into the subscription and paste
the `sk-ant-oat01-...` token. Before storing anything, the server checks the
format and then sends a one-token request to the Anthropic API:

- a malformed or rejected token is refused with `422`, and nothing is stored;
- a rate-limited token (`429`) is stored and shown as limited until its window resets;
- an unreachable provider gives `502`.

## Where the credential lives

The credential is stored as the system secret `accounts/<id>/credential`,
encrypted like every other secret. That namespace is hidden from the Secrets
page and refused by the Secrets API. No response, log, audit entry or event
carries the token.

## Injection per transport

| Transport | How the token reaches the CLI |
|---|---|
| Local (`ClaudeCodeProvider`) | `CLAUDE_CODE_OAUTH_TOKEN` in the child process environment |
| Docker (`DockerProvider`) | `CLAUDE_CODE_OAUTH_TOKEN` in the exec environment |
| SSH (`SshProvider`) | first line of stdin, read by the remote shell and exported |
| Kubernetes | not yet: agent steps use the pod environment |

The token never appears on a command line. The worker forces the CLI into
`stream-json` mode so it can read the `rate_limit_event` lines that report the
windows.

## Selection

The worker keeps only available accounts: enabled, token not rejected, not
expired, no applicable window rejected until its reset, and under
`max_concurrency`. A window scoped to a model family (for example the Opus
7 day window) only blocks steps using that family. A strategy then picks one:

| Strategy | Picks |
|---|---|
| `least_utilized` (default) | the lowest peak utilization, plus 0.15 per running step |
| `priority` | the lowest `priority` value |
| `round_robin` | the next account, by name |

Choose it on the worker:

```rust,ignore
use ironflow_core::account_strategy::Priority;

let worker = WorkerBuilder::new(&api_url, &worker_token)
    .provider(Arc::new(ClaudeCodeProvider::new()))
    .account_strategy(Arc::new(Priority))
    .build()?;
```

## Targeting an account or a pool

By default a step may use any account of the kind. A step can narrow that:

```rust,ignore
use std::time::Duration;

// Only accounts tagged `team`.
let review = AgentStepConfig::new("Review the diff").account_pool("team");
// Exactly this account: never another one, never the worker's own token.
let audit = AgentStepConfig::new("Audit the release").account("perso-max");
```

A name that matches no account fails the step with `AgentError::AccountNotFound`.
A pool with no account fails it with `AgentError::NoCapacity`.

## When every account is limited

If the CLI reports a rejected window while a step runs on an account, the
worker records it and runs the step again on the next available account. A
step pinned with `.account(..)` has no other account and does not fail over.

When no targeted account is available, the run is not failed: it goes
`Sleeping` until the earliest window reset, and the step runs again from the
start when the run wakes. The run shows the provider kind it waits for
(`capacity_wait_kind` in the API, a line in `ironflow run get <id>`, a note in the
dashboard) and its wake-up time in `scheduled_at`. Adding an account of that
kind, re-enabling one or renewing its token wakes the run right away. An
account that is only saturated (`max_concurrency` reached) is tried again
after a minute.

The wait is bounded, counting every time the same step was parked: 6 hours by
default. A reset further away fails the step with `AgentError::NoCapacity`,
which carries the next reset time. Set the bound per step or on the worker; the
step wins:

```rust,ignore
// Fail fast instead of waiting.
let urgent = AgentStepConfig::new("Triage the incident").fail_fast_on_capacity();

let worker = WorkerBuilder::new(&api_url, &worker_token)
    .provider(Arc::new(ClaudeCodeProvider::new()))
    .max_capacity_wait(Duration::from_secs(2 * 3600))
    .build()?;
```

A step marked `allow_failure()` still sleeps: waiting for capacity is not a
failure. With no account at all, a rate limit reported on the worker's own
token puts the run to sleep the same way.

## CLI

```sh
claude setup-token | ironflow accounts add perso-max --token-stdin --tag perso --priority 10
ironflow accounts list
ironflow accounts usage perso-max
ironflow accounts update perso-max --max-concurrency 2
ironflow accounts test perso-max
ironflow accounts remove perso-max --yes
```

Usage history is kept `PROVIDER_ACCOUNT_USAGE_RETENTION_DAYS` days (30 by default).
