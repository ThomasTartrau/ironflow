# Roadmap

What is built, what is being worked on, what is planned, and what was turned down.

GitLab issues are the source of truth for status, priority and discussion. This file
is the versioned summary: it says where the project stands without requiring anyone to
read the issue tracker. Every entry links to its issue.

An entry only moves to **Done** once the code is merged into `main`.

## Done

What the platform already does is documented in the Features section of
[README.md](README.md), and the
per-release detail is in [CHANGELOG.md](CHANGELOG.md). This section only records the
milestones, so nothing here has to be kept in sync with either.

- **v1** - agent and shell operations, retry policies, remote transports (SSH, Docker,
  Kubernetes).
- **v2 core** - workflow engine (FSM, DAG, approvals, sub-workflows), REST API, dashboard,
  auth, store.
- **v2 platform** - secrets, audit log, live log streaming, notifications, Prometheus
  metrics, run labels and scheduling.
- **v2 providers** - HTTP providers beyond the Claude CLI (Anthropic, OpenAI, Mistral,
  Gemini, NVIDIA NIM) with an agentic tool loop and an MCP bridge.
- **v2 run reliability** - worker leases and reaper, automatic retry, idempotency keys,
  cost caps, run authorship.
- **v2 client surface** - `ironflow-sdk`, `ironflow-cli`, `ironflow-mcp`.

## In progress

- Versioned roadmap and implementation notes in the repo - [#22](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/22)

## Planned

- Multi-tenancy: organisations and RBAC - [#6](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/6)
- Artifact persistence and passing between steps - [#7](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/7)
- Prebuilt, shareable workflow templates - [#8](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/8)
- Metrics and observability - partially shipped: the Prometheus endpoint and run/step
  counters exist; OpenTelemetry traces, `ironflow_run_cost_usd_total` and
  `ironflow_worker_queue_depth` do not - [#9](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/9)
- Complete the CLI: reject, secrets, api-keys, users, audit-logs - [#20](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/20)
- Secret encryption key rotation - [#21](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/21)
- Use `handler_version` for replay compatibility - [#23](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/23)
- Message-queue and run-event triggers - [#24](https://gitlab.com/ThomasTartrau/ironflow/-/work_items/24)

## Rejected

Ideas that were considered and turned down, each with the reason, so the same ones do not
come back every six months.

<!-- Format: - <idea> - <one-sentence reason>. Link the issue if there is one. -->

Nothing rejected yet.
