<div align="center">

```
  ___                  __ _
 |_ _|_ __ ___  _ __ / _| | _____      __
  | || '__/ _ \| '_ \| |_| |/ _ \ \ /\ / /
  | || | | (_) | | | |  _| | (_) \ V  V /
 |___|_|  \___/|_| |_|_| |_|\___/ \_/\_/
```

# Ironflow

[![pipeline status](https://img.shields.io/gitlab/pipeline-status/ThomasTartrau%2Fironflow?branch=main&style=for-the-badge&logo=gitlab&logoColor=white)](https://gitlab.com/ThomasTartrau/ironflow/-/pipelines)
[![ironflow-core](https://img.shields.io/crates/v/ironflow-core.svg?style=for-the-badge&logo=rust&logoColor=white&label=ironflow-core)](https://crates.io/crates/ironflow-core)
[![ironflow-runtime](https://img.shields.io/crates/v/ironflow-runtime.svg?style=for-the-badge&logo=rust&logoColor=white&label=ironflow-runtime)](https://crates.io/crates/ironflow-runtime)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg?style=for-the-badge)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-1.94+-orange?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)

**Workflows as imperative Rust code - no YAML, no DSL.**

**Claude Code native agent support with structured JSON output.**

*Shell commands • HTTP requests • AI agents • Webhooks • Cron scheduling*

[Getting Started](#-quick-start) •
[Operations](#-operations) •
[Runtime](#-runtime-webhooks--cron) •
[Examples](#-use-cases)

</div>

---

## ✨ Features

| | |
|---|---|
| | |
|---|---|
| **🦀 Imperative API** - A workflow is an `async fn`, not a config file | **🤖 AI Agent** - Claude Code in headless mode, invoked via CLI |
| **🎯 Type-safe output** - Derive `JsonSchema` on your types, get typed responses | **💰 Budget control** - Per-step `max_budget_usd` prevents runaway costs |
| **🧪 Record/Replay** - Deterministic agent tests without spending tokens | **❌ No retry logic** - A step fails, the workflow fails. Simple and predictable |
| **🔀 Parallel execution** - `try_join_all` with optional concurrency limits | **🌐 Webhook auth** - GitHub, GitLab, HMAC-SHA256, static header |
| **⏰ Cron scheduling** - Job scheduling via `tokio-cron-scheduler` | **📊 Prometheus metrics** - Shell, HTTP, agent, webhook, and cron counters |
| **🏃 Dry-run mode** - Skip execution while logging intent | **📈 Workflow tracker** - Cost, tokens, and duration across steps |
| **🚀 Remote transports** - Run Claude Code via SSH, Docker, or Kubernetes | **🔑 OAuth support** - Inject Claude OAuth credentials into remote pods |

---

## 🏗️ Architecture

```
ironflow/
├── ironflow-core         # Operations: Shell, Http, Agent
│   ├── operations/       #   Shell, Http, Agent builders + IntoFuture
│   ├── providers/
│   │   ├── claude/       #   Local, SSH, Docker, K8s transports
│   │   └── record_replay #   RecordReplayProvider (test fixtures)
│   ├── tracker.rs        #   WorkflowTracker (cost/tokens/duration)
│   ├── parallel.rs       #   try_join_all, try_join_all_limited
│   └── dry_run.rs        #   Global + per-operation dry-run control
│
├── ironflow-runtime      # Daemon layer (depends on ironflow-core)
│   ├── runtime.rs        #   Runtime builder + axum server
│   ├── webhook.rs        #   WebhookAuth (None, Header, HmacSha256, GitHub, GitLab)
│   └── cron.rs           #   Cron job scheduling (tokio-cron-scheduler)
```

`ironflow-core` is standalone. `ironflow-runtime` depends on `ironflow-core` and adds webhook + cron triggering with an HTTP server.

---

## ⚡ Quick Start

Add the crates to your project:

```bash
cargo add ironflow-core
# Optional: add the runtime for webhooks and cron
cargo add ironflow-runtime
```

Minimal example:

```rust
use ironflow_core::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ClaudeCodeProvider::new();

    // Run a shell command
    let files = Shell::new("ls -la src/").await?;

    // Feed the output into an Agent
    let review = Agent::new()
        .prompt(&format!("Review these source files:\n{}", files.stdout()))
        .model(Model::Sonnet)
        .max_budget_usd(0.10)
        .run(&provider)
        .await?;

    println!("{}", review.text());
    Ok(())
}
```

---

## 🛠️ Operations

### Shell

Run system commands with timeout, working directory, and environment control. Implements `IntoFuture` so you can `await` directly.

```rust
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

Perform HTTP requests with headers, JSON body, and timeout. Non-2xx status codes are not treated as errors - use `is_success()` to check.

```rust
use ironflow_core::prelude::*;
use std::time::Duration;

# async fn example() -> Result<(), OperationError> {
let output = Http::post("https://httpbin.org/post")
    .header("Authorization", "Bearer token123")
    .json(&serde_json::json!({"key": "value"}))
    .timeout(Duration::from_secs(30))
    .await?;

println!("status: {}, body: {}", output.status(), output.body());
# Ok(())
# }
```

### Agent

Invoke Claude Code (or any `AgentProvider`) with builder configuration. Supports structured output via `JsonSchema`.

```rust
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
    .model(Model::Opus)
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
<summary><b>🔄 Session Resume</b></summary>

Continue a multi-turn conversation by passing the session ID from a previous result:

```rust
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

---

## 🚀 Remote Transports

Run Claude Code on remote machines instead of locally. Each transport is opt-in via feature flags.

```bash
cargo add ironflow-core --features transport-ssh
cargo add ironflow-core --features transport-docker
cargo add ironflow-core --features transport-k8s
```

### SSH

Execute Claude Code on a remote host via SSH:

```rust
use ironflow_core::prelude::*;
use ironflow_core::providers::claude::SshProvider;

# async fn example() -> Result<(), OperationError> {
let provider = SshProvider::new("build-server.example.com", "deploy")
    .password("s3cret")
    .working_dir("/opt/project");

let result = Agent::new()
    .prompt("Review the codebase")
    .run(&provider)
    .await?;
# Ok(())
# }
```

### Docker

Execute Claude Code inside a running Docker container:

```rust
use ironflow_core::prelude::*;
use ironflow_core::providers::claude::DockerProvider;

# async fn example() -> Result<(), OperationError> {
let provider = DockerProvider::new("claude-worker")
    .user("node")
    .working_dir("/workspace");

let result = Agent::new()
    .prompt("Review the codebase")
    .run(&provider)
    .await?;
# Ok(())
# }
```

### Kubernetes

Two modes: ephemeral (one pod per invocation) and persistent (reuses a worker pod).

```rust
use ironflow_core::prelude::*;
use ironflow_core::providers::claude::{
    K8sEphemeralProvider, K8sPersistentProvider, K8sClusterConfig, ImagePullPolicy,
};

# async fn example() -> Result<(), OperationError> {
// Ephemeral: creates a pod, runs claude, deletes the pod
let ephemeral = K8sEphemeralProvider::new("my-registry/claude:v1")
    .namespace("ci")
    .image_pull_policy(ImagePullPolicy::IfNotPresent)
    .oauth_credentials(r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-..."}}"#);

// Persistent: reuses a long-running worker pod
let persistent = K8sPersistentProvider::new("my-registry/claude:v1")
    .pod_name("claude-worker")
    .namespace("ci")
    .oauth_credentials(r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-..."}}"#);

// Custom kubeconfig (file or inline YAML)
let provider = K8sEphemeralProvider::new("my-registry/claude:v1")
    .cluster_config(K8sClusterConfig::KubeconfigFile("/path/to/kubeconfig".into()));

let result = Agent::new()
    .prompt("Review the codebase")
    .run(&ephemeral)
    .await?;
# Ok(())
# }
```

<details>
<summary><b>Transport comparison</b></summary>

| Transport | Feature flag | Use case |
|-----------|-------------|----------|
| `ClaudeCodeProvider` | *(always available)* | Claude CLI installed locally |
| `SshProvider` | `transport-ssh` | Remote build server or GPU instance |
| `DockerProvider` | `transport-docker` | Isolated container with credentials |
| `K8sEphemeralProvider` | `transport-k8s` | Full isolation, one pod per invocation |
| `K8sPersistentProvider` | `transport-k8s` | Low latency, reuses worker pod |

</details>

---

## 🔀 Parallel Execution

### Static parallelism (known number of steps)

Use `tokio::try_join!` when you know at compile time how many steps to run:

```rust
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
let (files, status) = tokio::try_join!(
    Shell::new("ls -la"),
    Shell::new("git status"),
)?;
# Ok(())
# }
```

### Dynamic parallelism (runtime-determined)

Use `try_join_all` when the number of steps is determined at runtime:

```rust
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
let commands = vec!["ls -la", "git status", "df -h"];
let results = try_join_all(
    commands.iter().map(|cmd| Shell::new(cmd).run())
).await?;

for (cmd, output) in commands.iter().zip(&results) {
    println!("{cmd}: {}", output.stdout());
}
# Ok(())
# }
```

<details>
<summary><b>🔒 Concurrency-limited parallelism</b></summary>

Use `try_join_all_limited` to cap the number of concurrent operations:

```rust
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
let provider = ClaudeCodeProvider::new();
let prompts = vec!["Summarize file A", "Summarize file B", "Summarize file C"];

let results = try_join_all_limited(
    prompts.iter().map(|p| {
        Agent::new()
            .prompt(p)
            .model(Model::Haiku)
            .max_budget_usd(0.10)
            .run(&provider)
    }),
    2, // at most 2 agent calls at a time
).await?;
# Ok(())
# }
```

</details>

---

## 🌐 Runtime (Webhooks + Cron)

The `ironflow-runtime` crate provides an HTTP server with webhook endpoints and cron scheduling.

```rust
use ironflow_core::prelude::*;
use ironflow_runtime::prelude::*;

async fn on_push(payload: serde_json::Value, provider: &ClaudeCodeProvider) {
    let branch = payload["ref"].as_str().unwrap_or("main");
    let diff = Shell::new(&format!("git diff origin/main...origin/{branch}"))
        .await
        .unwrap();
    let review = Agent::new()
        .prompt(&format!("Review this diff:\n{}", diff.stdout()))
        .model(Model::Sonnet)
        .max_budget_usd(0.50)
        .run(provider)
        .await
        .unwrap();
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

### Webhook Authentication

| Method | Usage |
|--------|-------|
| `WebhookAuth::none()` | No authentication |
| `WebhookAuth::header(name, value)` | Static header comparison |
| `WebhookAuth::github(secret)` | GitHub HMAC-SHA256 (`X-Hub-Signature-256`) |
| `WebhookAuth::gitlab(secret)` | GitLab token (`X-Gitlab-Token`) |

### Built-in Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Returns `200 OK` with body `"ok"` |
| `GET` | `/metrics` | Prometheus metrics (requires `prometheus` feature) |

---

## 📊 Prometheus Metrics

Enable the `prometheus` feature flag to expose operational metrics:

```toml
[dependencies]
ironflow-core = { version = "0.1", features = ["prometheus"] }
ironflow-runtime = { version = "0.1", features = ["prometheus"] }
```

When using `ironflow-runtime` with the `prometheus` feature, a `/metrics` endpoint is automatically registered.

<details>
<summary><b>📋 Exposed Metrics</b></summary>

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
| `ironflow_runs_reaped_total` | Counter | `outcome` |
| `ironflow_worker_leases_lost_total` | Counter | |

</details>

---

## 🔒 Worker Leases and Recovery

A worker that dies mid-execution (OOM, reclaimed spot instance, deleted pod)
would otherwise leave its run stuck in `Running` forever: workers only pick up
`Pending` runs, so nobody takes it over.

Every run a worker picks up carries a **lease**: the worker identifies itself
(`worker_id`) and refreshes an expiry (`lease_expires_at`) every 30 seconds
while it executes. The lease lasts 90 seconds, so three missed refreshes make
the run recoverable. If the worker cannot refresh it — the API took the run away,
or the API stayed unreachable longer than the lease — the worker **abandons** the
run instead of executing it twice.

The **reaper** runs on the API side and requeues those runs:

```rust
use ironflow_api::reaper::Reaper;
use tokio_util::sync::CancellationToken;

let shutdown = CancellationToken::new();
tokio::spawn(Reaper::new(store, engine).run(shutdown.clone()));
```

**You must start the reaper yourself.** Without it, leases expire and nothing
requeues the runs — the original problem is unchanged.

Every 60 seconds the reaper recovers at most 100 runs whose lease expired: each
goes back to `Pending` with `retry_count` incremented, or to `Failed` with
`worker lease expired` once `max_retries` is exhausted. A recovered run restarts
from scratch — completed steps are not replayed. Runs are recovered at most once
even with several API instances, and a run holding a valid lease is never touched.

Defaults are adjustable:

```rust
let reaper = Reaper::new(store, engine)
    .interval(Duration::from_secs(30))
    .batch_size(50);

let worker = WorkerBuilder::new(api_url, token)
    .worker_id("worker-eu-west-1a")     // default: worker-<uuid>, new at every start
    .lease_ttl(Duration::from_secs(120))
    .lease_refresh_interval(Duration::from_secs(30))
    .build()?;
```

> **Note** — Runs executed inside the API server (inline execution, resume after
> a human approval) hold no lease and are never reaped. An API crash during such
> a run still leaves it stuck.

---

## 📈 WorkflowTracker

Track cost, tokens, and duration across all steps of a workflow:

```rust
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

tracker.summary(); // emits structured log via tracing
println!("Total cost: ${:.4}", tracker.total_cost_usd());
println!("Steps: {}", tracker.step_count());
# Ok(())
# }
```

---

## 🏃 Dry-Run Mode

Skip execution while logging intent. Useful for testing workflow logic without side effects.

```rust
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
// Global dry-run: all operations skip execution
set_dry_run(true);
let output = Shell::new("rm -rf /").await?; // not executed
assert_eq!(output.stdout(), "");

// Per-operation dry-run (overrides global)
set_dry_run(false);
let output = Shell::new("echo hello").dry_run(true).await?;
assert_eq!(output.stdout(), "");
# Ok(())
# }
```

---

## 🧪 Record/Replay Testing

Test agent workflows deterministically without spending tokens. The `RecordReplayProvider` wraps any `AgentProvider` and saves/loads responses from JSON fixtures.

```rust
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
let inner = ClaudeCodeProvider::new();

// Record mode: calls the real provider and saves responses
// Activated by setting IRONFLOW_RECORD=1 env var
let provider = RecordReplayProvider::new(inner, "tests/fixtures");

// Or force replay mode (ignores IRONFLOW_RECORD env var)
let inner = ClaudeCodeProvider::new();
let provider = RecordReplayProvider::replay(inner, "tests/fixtures");

let result = Agent::new()
    .prompt("Explain ownership in Rust")
    .max_budget_usd(0.10)
    .run(&provider)
    .await?;
# Ok(())
# }
```

Fixture files are named by a hash of the prompt + system prompt + JSON schema, so identical configurations always map to the same file.

---

## 💡 Use Cases

### 🔍 Automated Code Review

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐     ┌──────────────┐
│   GitLab    │────▶│  Get Diff &  │────▶│ Claude Code │────▶│    Post      │
│   Webhook   │     │    Files     │     │   Review    │     │   Comments   │
└─────────────┘     └──────────────┘     └─────────────┘     └──────────────┘
```

<details>
<summary><b>Workflow Steps</b></summary>

1. **Trigger**: GitLab/GitHub webhook on new MR/PR
2. **Fetch**: Get changed files and diff via API
3. **Review**: Agent analyzes code for bugs, security issues, improvements
4. **Post**: Send review comments back to GitLab/GitHub

</details>

### 📚 Auto Documentation

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐     ┌──────────────┐
│    Code     │────▶│  Get Changed │────▶│ Claude Code │────▶│   Commit     │
│    Push     │     │    Files     │     │  Gen Docs   │     │    Docs      │
└─────────────┘     └──────────────┘     └─────────────┘     └──────────────┘
```

<details>
<summary><b>Workflow Steps</b></summary>

1. **Trigger**: Webhook on code push to main
2. **Identify**: Get list of changed files
3. **Generate**: Agent generates documentation with descriptions, params, examples
4. **Commit**: Create commit with updated docs

</details>

### 🐛 Auto Bug Fixing

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐     ┌──────────────┐
│   Sentry    │────▶│    Parse     │────▶│ Claude Code │────▶│  Create PR   │
│   Alert     │     │ Stack Trace  │     │   Fix Bug   │     │   + Notify   │
└─────────────┘     └──────────────┘     └─────────────┘     └──────────────┘
```

<details>
<summary><b>Workflow Steps</b></summary>

1. **Trigger**: Webhook from error monitoring (Sentry, Datadog)
2. **Analyze**: Parse error stack trace
3. **Fix**: Agent analyzes and fixes the bug
4. **Test**: Run tests to validate
5. **PR**: Create pull request with fix

</details>

---

## ⚙️ Configuration

Ironflow uses `.env` files via [dotenvy](https://crates.io/crates/dotenvy). The runtime loads `.env` automatically when `serve()` is called.

```env
GITLAB_SECRET=my-webhook-secret
GITHUB_SECRET=my-github-secret
```

---

## 📄 License

MIT License - see [LICENSE](LICENSE) for details.

---

<div align="center">

**[GitLab](https://gitlab.com/ThomasTartrau/ironflow)** •
**[ironflow-core](https://crates.io/crates/ironflow-core)** •
**[ironflow-runtime](https://crates.io/crates/ironflow-runtime)**

</div>
