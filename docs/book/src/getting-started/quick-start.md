# Quick Start

This page takes you from nothing to a running workflow in three stages: run the demo, drive
it from a terminal, then start your own project.

## 1. Run the demo

You need [Rust 1.94+](https://rustup.rs), [Node 22+ with pnpm](https://pnpm.io/installation)
for the dashboard, and, for agent steps, the
[Claude Code CLI](https://docs.claude.com/en/docs/claude-code).

```bash
git clone https://gitlab.com/ThomasTartrau/ironflow.git
cd ironflow
```

Build the dashboard first: the API server embeds `ironflow-dashboard/dist/` when it compiles.

```bash
(cd ironflow-dashboard && pnpm install && pnpm build)
```

Start the API server. It serves the REST API and the dashboard on
<http://localhost:3000>, with a dozen example workflows already registered:

```bash
IRONFLOW_ENV=development cargo run -p ironflow-example-server
```

`IRONFLOW_ENV=development` lets the server boot without secrets: it generates them and logs
the worker token (`start workers with WORKER_TOKEN=...`).

In a second terminal, start a worker. The server stores runs; the worker is the process that
executes them.

```bash
WORKER_TOKEN=<token from the server log> cargo run -p ironflow-example-worker
```

Open <http://localhost:3000>, create an account, open **deploy-approval** and click **Run**.
The run executes its steps, then stops on an approval gate: approve it and the last step runs.

The example server keeps runs in memory, so they are gone when it stops. See
[Running the Server](server.md) to use Postgres.

## 2. Drive it from a terminal

Create an API key in the dashboard, under **API keys**, then:

```bash
cargo install ironflow-cli

export IRONFLOW_URL=http://localhost:3000
export IRONFLOW_API_KEY=irfl_...

ironflow-cli workflow list
ironflow-cli run create ci-pipeline
ironflow-cli logs <run-id>
```

The CLI, the Rust SDK and the MCP server are described in
[Interfaces](../reference/interfaces.md).

## 3. Start your own project

A project has three parts: a library with your workflows, a server binary and a worker
binary. [Installation](installation.md) shows the layout, and the example
[server](server.md) and [worker](worker.md) are the code to start from.

With Claude Code, the [Ironflow plugin](../reference/interfaces.md#claude-code-plugin)
scaffolds that project for you with `/ironflow setup`.

Then write your first workflow with [Writing a Workflow](../guides/writing-a-workflow.md).
