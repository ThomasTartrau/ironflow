# Ironflow

A workflow orchestration platform where workflows are imperative Rust code executed by background workers, with persistence, cost tracking, and human approval gates.

## Language

### Orchestration

**Workflow**:
A named handler (impl `WorkflowHandler`) registered in the Engine. It declares steps as imperative async Rust code. A Workflow exists only in memory -- it is never persisted. Only its name appears on the Runs it produces.
_Avoid_: pipeline, job definition, template

**Run**:
A single execution of a Workflow, persisted in the Store. Tracks status (FSM), cost, duration, retry count, and the Trigger that started it. Identified by a UUIDv7.
_Avoid_: execution, invocation, job

**Step**:
An atomic unit of work within a Run, persisted in the Store. Records input, output, status (FSM), cost, duration, and token counts. Each Step has a kind: Shell, Http, Agent, Approval, Workflow (sub-workflow), or Custom.
_Avoid_: task, action, stage

**Trigger**:
How a Run was started. Covers both external sources (Webhook, Cron, Nats, Polling, Api, Manual) and internal origins (Retry, Workflow, RunEvent).
_Avoid_: source, origin, event source

**Approval**:
A human gate that suspends a Run until someone approves or rejects it. Modeled as a Step with kind Approval. Rejection is terminal for the Step; the handler decides what happens to the Run.
_Avoid_: gate, checkpoint, review

**Artifact**:
A file produced by a Step. Metadata (name, content type, SHA-256, size) is persisted in the Store; the bytes live in a blob store.
_Avoid_: output file, attachment, asset

**Label**:
A user-defined key-value pair on a Run, used for categorization and filtering. Workflows can declare default Labels applied to every Run they produce.
_Avoid_: tag, annotation, metadata

### Execution

**Engine**:
The in-memory registry that maps Workflow names to handlers and orchestrates Run execution. Holds references to the Store, the Provider, and the event publisher.
_Avoid_: orchestrator, scheduler, dispatcher

**Worker**:
A background process that polls the API for pending Runs, acquires a Lease, and executes the Workflow handler via the Engine. Scaling means starting more Workers.
_Avoid_: executor, runner, consumer

**Lease**:
A time-limited lock a Worker holds on a Run while executing it. The Worker refreshes the Lease periodically. If the Lease expires (worker crash, eviction), the Reaper recovers the Run.
_Avoid_: lock, reservation, claim

**Reaper**:
A background task in the API server that periodically finds Runs with expired Leases and requeues them (back to Pending if retries remain, Failed otherwise).
_Avoid_: garbage collector, watchdog, janitor

**Provider**:
A backend that executes Agent steps. Implements the `AgentProvider` trait. Two families: process-based (ClaudeCode, Docker, SSH, K8s) and HTTP-based (Anthropic, OpenAI, Gemini, Mistral).
_Avoid_: backend, executor, adapter (adapter is an internal implementation detail of HTTP-based providers)

**Operation**:
The extensibility mechanism for custom Step types. A trait with `kind()` and `execute()`, invoked via `ctx.operation()`. Built-in step types (Shell, Http, Agent, Approval) are not Operations -- they have dedicated methods on WorkflowContext.
_Avoid_: plugin, extension, custom step type

### Identity & access

**User**:
A registered account with email, username, and password (Argon2id). The first User created is admin; subsequent ones are members.
_Avoid_: account, member (member is a role, not the entity)

**API Key**:
A scoped, hashed token owned by a User for machine-to-machine authentication. Carries a set of Scopes that limit what it can do.
_Avoid_: token, service account, credential

**Scope**:
A permission granted to an API Key (e.g. RunsRead, RunsWrite, RunsManage, Admin). A Key with no Scopes has no permissions.
_Avoid_: permission, role, grant

**Secret**:
An encrypted key-value pair stored in the database, namespaced by Workflow. Decrypted at read time, injected into Steps at execution.
_Avoid_: credential, env var, config value

### Observability

**Event**:
A domain event emitted during the lifecycle (RunCreated, RunStatusChanged, StepCompleted, etc.). Consumed by subscribers for SSE streaming, webhook notifications, and audit logging.
_Avoid_: notification, message, signal

**Audit Log Entry**:
A persisted Event with denormalized context IDs (run, step, user) for compliance and post-mortem filtering.
_Avoid_: event log, activity log, history
