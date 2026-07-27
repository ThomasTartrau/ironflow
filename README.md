<div align="center">

```text
  ___                  __ _
 |_ _|_ __ ___  _ __ / _| | _____      __
  | || '__/ _ \| '_ \| |_| |/ _ \ \ /\ / /
  | || | | (_) | | | |  _| | (_) \ V  V /
 |___|_|  \___/|_| |_|_| |_|\___/ \_/\_/
```

# Ironflow

[![pipeline status](https://img.shields.io/gitlab/pipeline-status/ThomasTartrau%2Fironflow?branch=main&style=for-the-badge&logo=gitlab&logoColor=white)](https://gitlab.com/ThomasTartrau/ironflow/-/pipelines)
[![ironflow-core](https://img.shields.io/crates/v/ironflow-core.svg?style=for-the-badge&logo=rust&logoColor=white&label=core)](https://crates.io/crates/ironflow-core)
[![ironflow-cli](https://img.shields.io/crates/v/ironflow-cli.svg?style=for-the-badge&logo=rust&logoColor=white&label=cli)](https://crates.io/crates/ironflow-cli)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=for-the-badge)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.94+-orange?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)

**A workflow orchestration platform where workflows are imperative Rust code - no YAML, no DSL.**

*REST API • Background workers • Web dashboard • CLI • Rust SDK • MCP server*

[Quick Start](#-quick-start) •
[Architecture](#%EF%B8%8F-architecture) •
[Features](#-features) •
[Providers](#-agent-providers) •
[Interfaces](#-interfaces)

</div>

---

## What is Ironflow?

Ironflow runs workflows written as plain `async` Rust functions. A workflow declares its steps -
shell commands, HTTP calls, AI agents, sub-workflows, human approval gates - and the engine
persists every one of them, tracks cost and duration, and exposes the result over a REST API.

It ships as two things you can use independently:

- **A library.** Add `ironflow-core` to a binary and compose operations directly. No server, no
  database.
- **A platform.** Run the API server, one or more workers, and the dashboard. Workflows are
  triggered from the CLI, the REST API, a webhook, or a cron schedule; runs are persisted in
  Postgres, streamed live over SSE, and paused on approval gates until a human clicks approve.

---

## 🏗️ Architecture

| Crate | Version | Role |
|---|---|---|
| [`ironflow-core`](https://crates.io/crates/ironflow-core) | ![](https://img.shields.io/crates/v/ironflow-core.svg?label=) | Operations (Shell, Http, Agent), agent providers, tracker, parallelism, dry-run |
| [`ironflow-store`](https://crates.io/crates/ironflow-store) | ![](https://img.shields.io/crates/v/ironflow-store.svg?label=) | Storage trait plus Postgres and in-memory backends, encrypted secrets |
| [`ironflow-engine`](https://crates.io/crates/ironflow-engine) | ![](https://img.shields.io/crates/v/ironflow-engine.svg?label=) | Workflow orchestration, FSM-driven run lifecycle, outbound notifications |
| [`ironflow-auth`](https://crates.io/crates/ironflow-auth) | ![](https://img.shields.io/crates/v/ironflow-auth.svg?label=) | JWT issuing and verification, Argon2 password hashing, axum extractors |
| [`ironflow-api`](https://crates.io/crates/ironflow-api) | ![](https://img.shields.io/crates/v/ironflow-api.svg?label=) | REST API: runs, workflows, stats, audit logs, secrets, API keys, SSE |
| [`ironflow-worker`](https://crates.io/crates/ironflow-worker) | ![](https://img.shields.io/crates/v/ironflow-worker.svg?label=) | Background worker that polls the API and executes workflow handlers |
| [`ironflow-runtime`](https://crates.io/crates/ironflow-runtime) | ![](https://img.shields.io/crates/v/ironflow-runtime.svg?label=) | Standalone daemon: webhook endpoints (axum) and cron scheduling |
| [`ironflow-types`](https://crates.io/crates/ironflow-types) | ![](https://img.shields.io/crates/v/ironflow-types.svg?label=) | Shared API envelope types (`ApiResponse`, `ErrorEnvelope`) |
| [`ironflow-sdk`](https://crates.io/crates/ironflow-sdk) | ![](https://img.shields.io/crates/v/ironflow-sdk.svg?label=) | Type-safe Rust client, types generated from the OpenAPI spec |
| [`ironflow-cli`](https://crates.io/crates/ironflow-cli) | ![](https://img.shields.io/crates/v/ironflow-cli.svg?label=) | `ironflow-cli` command: create runs, list workflows, stream logs, show stats |
| [`ironflow-mcp`](https://crates.io/crates/ironflow-mcp) | ![](https://img.shields.io/crates/v/ironflow-mcp.svg?label=) | MCP server exposing runs, workflows and approvals to AI assistants |
| `ironflow-dashboard` | - | React + Vite web UI, embedded into `ironflow-api` or served separately |

How they fit together at runtime:

```text
   CLI ─┐
   SDK ─┤
   MCP ─┼──▶  ironflow-api  ──▶  ironflow-store  ◀──  ironflow-worker
Webhook ─┤     (REST + SSE)       (Postgres)            (engine + providers)
   Cron ─┘          │                                          │
                    ▼                                          ▼
              ironflow-dashboard                     Claude Code / SSH / Docker
                                                     K8s / Anthropic / OpenAI ...
```

The API owns persistence and never executes anything. Workers poll the API for pending runs,
execute the workflow handler locally, and stream steps and logs back. Scaling out means starting
more workers.

`ironflow-runtime` is a separate, lighter path: a standalone daemon with webhook and cron
endpoints that calls `ironflow-core` operations directly, without a store or an API.

---

## ⚡ Quick Start

### As a library

```bash
cargo add ironflow-core tokio --features tokio/full
```

```rust,no_run
use ironflow_core::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ClaudeCodeProvider::new();

    // Run a shell command
    let files = Shell::new("ls -la src/").await?;

    // Feed the output into an agent
    let review = Agent::new()
        .prompt(&format!("Review these source files:\n{}", files.stdout()))
        .model(Model::SONNET)
        .max_budget_usd(0.10)
        .run(&provider)
        .await?;

    println!("{}", review.text());
    Ok(())
}
```

### As a platform

The workspace ships a runnable server and worker, preloaded with a dozen example workflows.
Requires Rust 1.94+, Node 22+ with pnpm for the dashboard, and, for agent steps, the
[Claude Code CLI](https://docs.claude.com/en/docs/claude-code).

```bash
git clone https://gitlab.com/ThomasTartrau/ironflow.git
cd ironflow
```

Build the dashboard first - the API embeds `ironflow-dashboard/dist/` at compile time:

```bash
cd ironflow-dashboard && pnpm install && pnpm build && cd ..
```

```bash
# Terminal 1 - API + embedded dashboard on http://localhost:3000
cargo run -p ironflow-example-server
```

```bash
# Terminal 2 - worker polling the API
cargo run -p ironflow-example-worker
```

Open <http://localhost:3000>, create an account, and trigger a workflow from the UI. To drive it
from the terminal instead, generate a key under **API keys**:

```bash
cargo install ironflow-cli

export IRONFLOW_URL=http://localhost:3000
export IRONFLOW_API_KEY=irfl_...

ironflow-cli workflow list
ironflow-cli run create ci-pipeline
ironflow-cli logs <run-id>
```

The example server uses the in-memory store, so runs are lost on restart. Switch to Postgres by
setting `DATABASE_URL` and enabling the `store-postgres` feature - migrations live in
`ironflow-store/migrations/`.

---

## ✨ Features

### Step types

| Kind | Method | Description |
|------|--------|-------------|
| Shell | `ctx.shell()` | Command with timeout, working directory, environment |
| HTTP | `ctx.http()` | Request with headers, JSON body, timeout |
| Agent | `ctx.agent()` | AI invocation with budget cap and structured output |
| Sub-workflow | `ctx.workflow()` | Run another handler as a step, cost included in the parent |
| Approval | `ctx.approval()` | Human gate: the run pauses until approved or rejected |
| Custom | `ctx.operation()` | Your own `Operation` implementation (GitLab, Slack, Gmail, ...) |

A workflow is a `WorkflowHandler` implementation. Control flow is plain Rust - `if`, `for`, `?` -
not a DAG description language:

```rust,no_run
use ironflow_engine::config::{ApprovalConfig, ShellConfig, StepConfig};
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{HandlerFuture, WorkflowHandler};

struct Deploy;

impl WorkflowHandler for Deploy {
    fn name(&self) -> &str {
        "deploy"
    }

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {
        Box::pin(async move {
            ctx.shell("build", ShellConfig::new("cargo build --release"))
                .await?;

            // Fan out: these three run concurrently, `true` means fail fast
            let checks = ctx
                .parallel(
                    vec![
                        ("test", StepConfig::Shell(ShellConfig::new("cargo test"))),
                        ("lint", StepConfig::Shell(ShellConfig::new("cargo clippy"))),
                        ("audit", StepConfig::Shell(ShellConfig::new("cargo audit"))),
                    ],
                    true,
                )
                .await?;

            if checks.is_empty() {
                return Ok(());
            }

            // The run suspends here until a human approves it
            ctx.approval("gate", ApprovalConfig::new("Ship to production?"))
                .await?;

            ctx.shell("deploy", ShellConfig::new("./deploy.sh")).await?;
            Ok(())
        })
    }
}
```

### Triggers

| Trigger | Source |
|---------|--------|
| `Manual` | CLI or a direct programmatic call |
| `Api` | `POST /api/v1/runs` |
| `Webhook { path }` | Incoming webhook, authenticated per route |
| `Cron { schedule }` | Cron expression declared by the handler via `schedule()` |
| `Retry { parent_run_id }` | Retry of a previously failed run |
| `Workflow` | Invoked as a sub-workflow step by a parent run |

Runs also carry an optional `scheduled_at`: set it at creation and the run stays pending until
that timestamp.

### Platform capabilities

| | |
|---|---|
| **🔀 DAG and parallelism** - `ctx.parallel()` with fail-fast or collect-all semantics | **✋ Human approval** - runs suspend, resume by replaying completed steps from cache |
| **🔐 Encrypted secrets** - AES-GCM at rest, resolved at step execution time | **🔑 Scoped API keys** - `workflows_read`, `runs_read`, `runs_write`, `runs_manage`, `stats_read`, `admin` |
| **📜 Audit logs** - every mutating action recorded with actor and target | **📡 Live streaming** - step and log events over SSE, consumed by the dashboard and `ironflow logs` |
| **🔔 Outbound notifications** - webhook and Betterstack subscribers with retry | **📊 Prometheus metrics** - shell, HTTP, agent, webhook and cron counters |
| **💰 Budget control** - per-step `max_budget_usd` caps agent spending | **🧪 Record/replay** - deterministic agent tests without spending tokens |
| **🏃 Dry-run mode** - skip execution while logging intent | **❌ No hidden retries** - a step fails, the run fails, unless you ask for a `RetryPolicy` |

---

## 🤖 Agent Providers

Every provider implements `AgentProvider`, so a workflow written against one runs against any
other. All of them except `ClaudeCodeProvider` are behind a feature flag.

| Provider | Feature flag | Use case |
|----------|-------------|----------|
| `ClaudeCodeProvider` | *(always available)* | Claude Code CLI installed locally |
| `SshProvider` | `transport-ssh` | Claude Code on a remote build server |
| `DockerProvider` | `transport-docker` | Claude Code inside a running container |
| `K8sEphemeralProvider` | `transport-k8s` | One pod per invocation, full isolation |
| `K8sPersistentProvider` | `transport-k8s` | Reuses a worker pod, lower latency |
| `AnthropicApiProvider` | `provider-anthropic-api` | Anthropic Messages API, no CLI needed |
| `OpenAiProvider` | `provider-openai` | OpenAI Chat Completions |
| `GeminiProvider` | `provider-gemini` | Google Gemini |
| `MistralProvider` | `provider-mistral` | Mistral |
| `NvidiaProvider` | `provider-nvidia` | NVIDIA NIM, 100+ models behind one API |

HTTP providers are used exactly like the local one:

```rust,no_run
use ironflow_core::prelude::*;
use ironflow_core::providers::http::{NvidiaModel, NvidiaProvider};

# async fn example() -> Result<(), OperationError> {
let provider = NvidiaProvider::from_env(); // reads NVIDIA_API_KEY

let result = Agent::new()
    .prompt("Summarize the changelog")
    .model(NvidiaModel::DEEPSEEK_V4_FLASH)
    .max_budget_usd(0.10)
    .run(&provider)
    .await?;
# Ok(())
# }
```

HTTP providers have no CLI to call tools for them, so tools are opt-in per feature: `tool-bash`,
`tool-read-file`, `tool-web-fetch`, `tool-web-search`, and `tool-mcp` to bridge any MCP server
into the agent's toolset.

### Routing between providers

`ProviderRouter` dispatches on the model name, so a single workflow can mix vendors:

```rust,no_run
use std::sync::Arc;
use ironflow_core::prelude::*;
use ironflow_core::providers::http::NvidiaProvider;

# async fn example() -> Result<(), OperationError> {
let claude = Arc::new(ClaudeCodeProvider::new());
let nvidia = Arc::new(NvidiaProvider::from_env());

let router = ProviderRouter::new(claude)
    .route(ProviderMatcher::ModelPrefix("nvidia/".into()), nvidia);

// Goes to Claude Code
let a = Agent::new().prompt("Review").model(Model::SONNET).run(&router).await?;

// Goes to NVIDIA
let b = Agent::new().prompt("Review").model("nvidia/deepseek-v4-flash").run(&router).await?;
# Ok(())
# }
```

<details>
<summary><b>Remote transport examples</b></summary>

```rust,no_run
use ironflow_core::prelude::*;
use ironflow_core::providers::claude::{
    DockerProvider, ImagePullPolicy, K8sEphemeralProvider, K8sPersistentProvider, SshProvider,
};

# async fn example() -> Result<(), OperationError> {
// Remote host over SSH
let ssh = SshProvider::new("build-server.example.com", "deploy")
    .password("s3cret")
    .working_dir("/opt/project");

// Running Docker container
let docker = DockerProvider::new("claude-worker")
    .user("node")
    .working_dir("/workspace");

// Kubernetes: one pod per invocation
let ephemeral = K8sEphemeralProvider::new("my-registry/claude:v1")
    .namespace("ci")
    .image_pull_policy(ImagePullPolicy::IfNotPresent)
    .oauth_credentials(r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-..."}}"#);

// Kubernetes: long-lived worker pod
let persistent = K8sPersistentProvider::new("my-registry/claude:v1")
    .pod_name("claude-worker")
    .namespace("ci");

let result = Agent::new().prompt("Review the codebase").run(&ssh).await?;
# Ok(())
# }
```

</details>

---

## 🖥️ Interfaces

### Dashboard

React + Vite UI covering the workflow catalog (with dynamic forms generated from each handler's
`input_schema`), run history with filters, live step and log streaming, approval and rejection,
secrets, API keys, users, and audit logs.

Two ways to serve it:

- **Embedded** - build `ironflow-api` with the `dashboard` feature and the compiled assets are
  baked into the binary via `rust-embed`.
- **From disk** - set `DASHBOARD_DIR` to a build output directory, which overrides the embedded
  copy.

### CLI

```bash
cargo install ironflow-cli
```

```console
$ ironflow-cli workflow list
┌───────────────────┬──────────┬─────────┐
│ Name              ┆ Category ┆ Version │
╞═══════════════════╪══════════╪═════════╡
│ ci-pipeline       ┆ -        ┆ -       │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌┤
│ deploy-approval   ┆ -        ┆ -       │
├╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌╌┼╌╌╌╌╌╌╌╌╌┤
│ greeting          ┆ examples ┆ -       │
└───────────────────┴──────────┴─────────┘

$ ironflow-cli run create ci-pipeline
┌──────────┬─────────────┬─────────┬──────────┬─────────┬─────────────────────┬─────────┐
│ ID       ┆ Workflow    ┆ Status  ┆ Duration ┆ Cost    ┆ Created             ┆ Started │
╞══════════╪═════════════╪═════════╪══════════╪═════════╪═════════════════════╪═════════╡
│ 019f9f50 ┆ ci-pipeline ┆ pending ┆ 0ms      ┆ $0.0000 ┆ 2026-07-26 16:44:19 ┆ -       │
└──────────┴─────────────┴─────────┴──────────┴─────────┴─────────────────────┴─────────┘

$ ironflow-cli logs 019f9f50-17c8-73b1-9288-b41cbed28d1a
$ ironflow-cli run list --status completed --workflow ci-pipeline
$ ironflow-cli run get <run-id> --verbose
$ ironflow-cli stats
```

Configuration is resolved in this order: command-line flags (`--url`, `--api-key`), then the
`IRONFLOW_URL` and `IRONFLOW_API_KEY` environment variables, then `~/.ironflow.toml`:

```toml
base_url = "http://localhost:3000"
api_key = "irfl_..."
```

Add `--json` to any command for machine-readable output.

### Rust SDK

Types are generated from `openapi.json` at build time, so the client cannot drift from the API.

```rust,no_run
use ironflow_sdk::IronflowClient;
use ironflow_sdk::types::CreateRunRequest;

# async fn example() -> Result<(), Box<dyn std::error::Error>> {
let client = IronflowClient::new("http://localhost:3000", "irfl_...");

let mut payload = serde_json::Map::new();
payload.insert("branch".to_string(), serde_json::json!("main"));

let created = client
    .create_run(&CreateRunRequest {
        workflow: "ci-pipeline".to_string(),
        payload: Some(payload),
        labels: None,
        scheduled_at: None,
        max_cost_usd: None,
    })
    .await?;

let detail = client.get_run(created.data.id).await?;
println!("status: {:?}", detail.data.run.status);
# Ok(())
# }
```

### MCP server

Lets an AI assistant list workflows, trigger runs, inspect results, and approve or reject pending
gates.

```bash
cargo install ironflow-mcp
claude mcp add ironflow --env IRONFLOW_URL=http://localhost:3000 --env IRONFLOW_API_KEY=irfl_... -- ironflow-mcp
```

Or declare it in `.mcp.json`:

```json
{
  "mcpServers": {
    "ironflow": {
      "command": "ironflow-mcp",
      "env": {
        "IRONFLOW_URL": "http://localhost:3000",
        "IRONFLOW_API_KEY": "irfl_..."
      }
    }
  }
}
```

Exposed tools: `list_workflows`, `get_workflow`, `list_runs`, `get_run`, `create_run`,
`approve_run`, `reject_run`, `cancel_run`, `retry_run`, `get_stats`.

---

## 🚩 Feature Flags

| Crate | Flag | Effect |
|-------|------|--------|
| `ironflow-core` | `prometheus` | Emit operation metrics |
| | `transport-ssh` | `SshProvider` (russh) |
| | `transport-docker` | `DockerProvider` (bollard) |
| | `transport-k8s` | `K8sEphemeralProvider`, `K8sPersistentProvider` (kube) |
| | `provider-anthropic-api` | Anthropic Messages API provider |
| | `provider-openai` | OpenAI provider |
| | `provider-gemini` | Google Gemini provider |
| | `provider-mistral` | Mistral provider |
| | `provider-nvidia` | NVIDIA NIM provider |
| | `tool-bash` | Bash tool for HTTP providers |
| | `tool-read-file` | File reading tool for HTTP providers |
| | `tool-web-fetch` | Web fetch tool for HTTP providers |
| | `tool-web-search` | Web search tool for HTTP providers |
| | `tool-mcp` | MCP bridge, exposes MCP servers as agent tools |
| `ironflow-store` | `store-memory` *(default)* | In-memory store, no persistence |
| | `store-postgres` | Postgres backend (sqlx) |
| | `secret-store` | AES-GCM encrypted secrets |
| | `openapi` | utoipa schemas for stored entities |
| `ironflow-api` | `dashboard` | Embed the built dashboard via `rust-embed` |
| | `sign-up` | Expose the self-service sign-up route |
| | `prometheus` | Expose `/metrics` |
| | `openapi` | Expose `/api/v1/openapi.json` |
| `ironflow-engine` | `prometheus` | Engine metrics |
| | `openapi` | utoipa schemas for engine types |
| `ironflow-worker` | `prometheus` | Worker metrics |
| | `heartbeat` | Periodic liveness reporting to the API |
| `ironflow-runtime` | `prometheus` | Webhook and cron metrics |
| `ironflow-types` | `openapi` | utoipa schemas for envelope types |
| `ironflow-sdk` | `rustls` *(default)* | reqwest with rustls |
| | `native-tls` | reqwest with the platform TLS stack |

---

## ⚙️ Configuration

Ironflow reads `.env` via [dotenvy](https://crates.io/crates/dotenvy).

### API server

| Variable | Required | Default |
|----------|----------|---------|
| `IRONFLOW_ENV` | no | `development` |
| `DATABASE_URL` | in production | - |
| `JWT_SECRET` | in production | development secret |
| `WORKER_TOKEN` | in production | development token |
| `IRONFLOW_SECRET_KEY` | no | unset, secret store disabled |
| `PORT` | no | `3000` |
| `ALLOWED_ORIGINS` | no | same-origin only |
| `DASHBOARD_DIR` | no | uses the embedded dashboard |
| `WEBHOOK_URL` | no | no outbound webhook |
| `RATE_LIMIT_AUTH` | no | `10` req/min |
| `RATE_LIMIT_GENERAL` | no | `60` req/min |

Starting in production without `DATABASE_URL`, `JWT_SECRET` or `WORKER_TOKEN` aborts at boot
rather than falling back to development defaults. `IRONFLOW_SECRET_KEY` is a hex-encoded AES-GCM
key; without it the secret store stays off and workflows reading secrets fail.

### Worker

| Variable | Required | Default |
|----------|----------|---------|
| `API_URL` | no | `http://localhost:3000` |
| `WORKER_TOKEN` | must match the API | development token |
| `CONCURRENCY` | no | `2` |
| `POLL_INTERVAL_SECS` | no | `2` |

### CLI and MCP

| Variable | Required | Default |
|----------|----------|---------|
| `IRONFLOW_URL` | yes | - |
| `IRONFLOW_API_KEY` | yes | - |

---

## 🛠️ Library Reference

Everything below applies to `ironflow-core` used standalone, without the API or a store.

### Shell

```rust,no_run
use ironflow_core::prelude::*;
use std::time::Duration;

# async fn example() -> Result<(), OperationError> {
let output = Shell::new("cargo test")
    .dir("/path/to/project")
    .timeout(Duration::from_secs(120))
    .env("RUST_LOG", "debug")
    .await?;

println!("stdout: {}", output.stdout());
println!("exit code: {}", output.exit_code());
# Ok(())
# }
```

### Http

Non-2xx statuses are not errors - check `is_success()`.

```rust,no_run
use ironflow_core::prelude::*;
use std::time::Duration;

# async fn example() -> Result<(), OperationError> {
let output = Http::post("https://httpbin.org/post")
    .header("Authorization", "Bearer token123")
    .json(serde_json::json!({"key": "value"}))
    .timeout(Duration::from_secs(30))
    .await?;

println!("status: {}, body: {}", output.status(), output.body());
# Ok(())
# }
```

### Agent

Derive `JsonSchema` on a type and the provider is constrained to return it.

```rust,no_run
use ironflow_core::prelude::*;

#[derive(Deserialize, JsonSchema)]
struct Review {
    score: u8,
    summary: String,
}

# async fn example() -> Result<(), OperationError> {
let provider = ClaudeCodeProvider::new();

let result = Agent::new()
    .system_prompt("You are a senior Rust reviewer.")
    .prompt("Review the codebase")
    .model(Model::OPUS)
    .allowed_tools(&["Read", "Grep"])
    .max_turns(5)
    .max_budget_usd(0.50)
    .output::<Review>()
    .run(&provider)
    .await?;

let review: Review = result.json().expect("schema-validated output");
println!("Score: {}/10 - {}", review.score, review.summary);
println!("Cost: ${:.4}", result.cost_usd().unwrap_or(0.0));
# Ok(())
# }
```

<details>
<summary><b>🔄 Session resume</b></summary>

```rust,no_run
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
let provider = ClaudeCodeProvider::new();

let first = Agent::new()
    .prompt("Analyze the src/ directory")
    .max_budget_usd(0.10)
    .run(&provider)
    .await?;

let session = first.session_id().expect("provider returned session ID");

let followup = Agent::new()
    .prompt("Now suggest improvements")
    .resume(session)
    .max_budget_usd(0.10)
    .run(&provider)
    .await?;
# Ok(())
# }
```

</details>

<details>
<summary><b>🔀 Parallel execution</b></summary>

`tokio::try_join!` when the step count is known at compile time:

```rust,no_run
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
let (files, status) = tokio::try_join!(
    Shell::new("ls -la"),
    Shell::new("git status"),
)?;
# Ok(())
# }
```

`try_join_all` when it is decided at runtime, `try_join_all_limited` to cap concurrency:

```rust,no_run
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
let provider = ClaudeCodeProvider::new();
let prompts = vec!["Summarize file A", "Summarize file B", "Summarize file C"];

let results = try_join_all_limited(
    prompts.iter().map(|p| {
        Agent::new()
            .prompt(p)
            .model(Model::HAIKU)
            .max_budget_usd(0.10)
            .run(&provider)
    }),
    2, // at most 2 agent calls at a time
).await?;
# Ok(())
# }
```

</details>

<details>
<summary><b>📈 WorkflowTracker</b></summary>

Cost, tokens and duration across steps, for library use without a store:

```rust,no_run
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
let provider = ClaudeCodeProvider::new();
let mut tracker = WorkflowTracker::new("deploy-pipeline");

let files = Shell::new("ls -la").await?;
tracker.record_shell("list-files", &files);

let review = Agent::new()
    .prompt("Review the project")
    .max_budget_usd(0.10)
    .run(&provider)
    .await?;
tracker.record_agent("code-review", &review);

tracker.summary(); // structured log via tracing
println!("Total cost: ${:.4}", tracker.total_cost_usd());
println!("Steps: {}", tracker.step_count());
# Ok(())
# }
```

</details>

<details>
<summary><b>🏃 Dry-run mode</b></summary>

```rust,no_run
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
// Global: every operation skips execution
set_dry_run(true);
let output = Shell::new("rm -rf /").await?; // not executed
assert_eq!(output.stdout(), "");

// Per-operation, overrides the global setting
set_dry_run(false);
let output = Shell::new("echo hello").dry_run(true).await?;
assert_eq!(output.stdout(), "");
# Ok(())
# }
```

</details>

<details>
<summary><b>🧪 Record/replay testing</b></summary>

`RecordReplayProvider` wraps any provider and stores responses as JSON fixtures, keyed by a hash
of prompt + system prompt + schema.

```rust,no_run
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
// Record mode when IRONFLOW_RECORD=1, replay otherwise
let provider = RecordReplayProvider::new(ClaudeCodeProvider::new(), "tests/fixtures");

// Or force replay, ignoring the env var
let provider = RecordReplayProvider::replay(ClaudeCodeProvider::new(), "tests/fixtures");

let result = Agent::new()
    .prompt("Explain ownership in Rust")
    .max_budget_usd(0.10)
    .run(&provider)
    .await?;
# Ok(())
# }
```

</details>

---

## 🌐 Standalone Runtime

`ironflow-runtime` is the no-database path: an axum server exposing webhook endpoints and cron
jobs that call operations directly.

```rust,no_run
use ironflow_core::prelude::*;
use ironflow_runtime::prelude::*;

async fn on_push(payload: serde_json::Value, provider: &ClaudeCodeProvider) {
    let branch = payload["ref"].as_str().unwrap_or("main");
    let diff = Shell::new(&format!("git diff origin/main...origin/{branch}"))
        .await
        .expect("git diff");
    let review = Agent::new()
        .prompt(&format!("Review this diff:\n{}", diff.stdout()))
        .model(Model::SONNET)
        .max_budget_usd(0.50)
        .run(provider)
        .await
        .expect("agent review");
    println!("{}", review.text());
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ClaudeCodeProvider::new();

    Runtime::new()
        .webhook("/hooks/github", WebhookAuth::github("my-secret"), {
            let p = provider.clone();
            move |payload| {
                let p = p.clone();
                async move { on_push(payload, &p).await }
            }
        })
        .cron("0 30 8 * * *", "daily-report", || async {
            println!("running daily report");
        })
        .serve("0.0.0.0:8080")
        .await?;

    Ok(())
}
```

| Webhook auth | Behaviour |
|--------------|-----------|
| `WebhookAuth::none()` | No authentication |
| `WebhookAuth::header(name, value)` | Static header comparison |
| `WebhookAuth::github(secret)` | GitHub HMAC-SHA256 (`X-Hub-Signature-256`) |
| `WebhookAuth::gitlab(secret)` | GitLab token (`X-Gitlab-Token`) |

Built-in endpoints: `GET /health`, and `GET /metrics` with the `prometheus` feature.

<details>
<summary><b>📊 Exposed metrics</b></summary>

| Metric | Type | Labels |
|--------|------|--------|
| `ironflow_shell_total` | Counter | `status` |
| `ironflow_shell_duration_seconds` | Histogram | |
| `ironflow_http_total` | Counter | `method`, `status` |
| `ironflow_http_duration_seconds` | Histogram | |
| `ironflow_agent_total` | Counter | `model`, `status` |
| `ironflow_agent_duration_seconds` | Histogram | `model` |
| `ironflow_agent_cost_usd_total` | Gauge | `model` |
| `ironflow_agent_tokens_input_total` | Counter | `model` |
| `ironflow_agent_tokens_output_total` | Counter | `model` |
| `ironflow_webhook_received_total` | Counter | `path`, `auth` |
| `ironflow_cron_runs_total` | Counter | `job` |

</details>

---

## 💡 Use Cases

<details>
<summary><b>🔍 Automated code review</b></summary>

```text
┌─────────────┐     ┌──────────────┐     ┌─────────────┐     ┌──────────────┐
│   GitLab    │────▶│  Get Diff &  │────▶│    Agent    │────▶│    Post      │
│   Webhook   │     │    Files     │     │   Review    │     │   Comments   │
└─────────────┘     └──────────────┘     └─────────────┘     └──────────────┘
```

Webhook on a new MR, fetch the diff, review it with an agent under a budget cap, post comments
back. The run, its cost and its logs stay queryable in the dashboard.

</details>

<details>
<summary><b>🚀 Deploy with an approval gate</b></summary>

```text
┌────────┐   ┌──────────────────┐   ┌──────────┐   ┌────────────┐
│  Build │──▶│ test │ lint │ audit │──▶│ Approval │──▶│ Production │
└────────┘   └──────────────────┘   └──────────┘   └────────────┘
                  (parallel)          (human)
```

Checks run concurrently, then the run suspends. A human approves from the dashboard, the CLI, or
an AI assistant through MCP, and execution resumes at the next step - completed steps replay from
cache instead of re-running.

</details>

<details>
<summary><b>🐛 Alert-driven bug fixing</b></summary>

```text
┌─────────────┐     ┌──────────────┐     ┌─────────────┐     ┌──────────────┐
│   Sentry    │────▶│    Parse     │────▶│    Agent    │────▶│  Create MR   │
│   Alert     │     │ Stack Trace  │     │   Fix Bug   │     │   + Notify   │
└─────────────┘     └──────────────┘     └─────────────┘     └──────────────┘
```

Webhook from error monitoring, parse the stack trace, let an agent produce a fix, run the tests,
open a merge request. Outbound notification subscribers report the outcome.

</details>

---

## 🧑‍💻 Development

```bash
cargo build                                  # Build the workspace
cargo test                                   # Unit and integration tests
cargo test -p ironflow-readme-tests --doc    # Check every Rust snippet in this README compiles
cargo doc --no-deps                          # Docs, must be warning-free
```

The dashboard lives in `ironflow-dashboard/` - see
[its README](ironflow-dashboard/README.md) for the frontend workflow.

---

## 📄 License

MIT - see [LICENSE](LICENSE).

---

<div align="center">

**[GitLab](https://gitlab.com/ThomasTartrau/ironflow)**

[core](https://crates.io/crates/ironflow-core) •
[store](https://crates.io/crates/ironflow-store) •
[engine](https://crates.io/crates/ironflow-engine) •
[auth](https://crates.io/crates/ironflow-auth) •
[api](https://crates.io/crates/ironflow-api) •
[worker](https://crates.io/crates/ironflow-worker) •
[runtime](https://crates.io/crates/ironflow-runtime) •
[types](https://crates.io/crates/ironflow-types) •
[sdk](https://crates.io/crates/ironflow-sdk) •
[cli](https://crates.io/crates/ironflow-cli) •
[mcp](https://crates.io/crates/ironflow-mcp)

</div>
