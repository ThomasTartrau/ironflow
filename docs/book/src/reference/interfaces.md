# Interfaces

Workflows are written in Rust, but runs can be started and followed from five places: the
web dashboard, the CLI, the Rust SDK, the MCP server for AI assistants, and the REST API they
all sit on. The Claude Code plugin helps you write the workflows themselves.

## Dashboard

A React web UI served by the API server. It covers the workflow catalog (with a form generated
from each handler's `input_schema`), run history with filters, live steps and logs, approval and
rejection, secrets, API keys, users and audit logs.

Two ways to serve it:

- **Embedded**: build `ironflow-api` with the `dashboard` feature and the compiled assets are
  baked into the binary via `rust-embed`.
- **From disk**: set `DASHBOARD_DIR` to a build output directory, which overrides the embedded
  copy.

## CLI

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

Add `--json` to any command for machine-readable output. `ironflow-cli --help` lists every
command.

## Rust SDK

Types are generated from `openapi.json` at build time, so the client cannot drift from the
API.

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
        max_retries: Some(2),
        max_cost_usd: Some(1.0),
        concurrency_key: None,
        priority: None,
        concurrency_limits: Vec::new(),
        worker_tags: Vec::new(),
    })
    .await?;

let detail = client.get_run(created.data.id).await?;
println!("status: {:?}", detail.data.run.status);
# Ok(())
# }
```

## MCP server

Lets an AI assistant list workflows, trigger runs, inspect results, and approve or reject
pending gates.

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

Exposed tools include `list_workflows`, `get_workflow`, `list_runs`, `get_run`, `create_run`,
`approve_run`, `reject_run`, `cancel_run`, `retry_run` and `get_stats`.

## Claude Code plugin

Skills that teach Claude Code how to build on Ironflow: scaffold a project, write a handler,
write a custom operation, test it end to end, and review a handler for the mistakes the
compiler cannot catch (side effects around approval gates, unstable step names, leaked
secrets).

```bash
# 1. Register the Ironflow repository as a plugin marketplace
claude plugin marketplace add https://gitlab.com/ThomasTartrau/ironflow.git

# 2. Install the plugin
claude plugin install ironflow@ironflow
```

```text
/ironflow setup              # workspace: workflows lib, server, worker, hello workflow, e2e test
/ironflow workflow deploy    # a WorkflowHandler with a typed input schema, registered
/ironflow operation slack    # a custom Operation tracked as a step
/ironflow test deploy        # Engine + InMemoryStore + record/replay test
/ironflow review             # the workflow reviewer agent
```

Every Rust snippet in the plugin is compiled in CI, and the project template is scaffolded and
built against each release. Details in the
[plugin README](https://gitlab.com/ThomasTartrau/ironflow/-/blob/main/plugins/ironflow/README.md).

## REST API

Everything above goes through the REST API under `/api/v1`. With the `openapi` feature, the
server publishes its specification at `/api/v1/openapi.json`.
