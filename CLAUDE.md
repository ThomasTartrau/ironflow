# Ironflow - Development Guidelines

## Project Structure

```
ironflow/
├── ironflow-core/       # Shell, Agent, AgentProvider trait, providers, tracker, utils
├── ironflow-runtime/    # Daemon: webhooks (axum), cron (tokio-cron-scheduler)
├── ironflow-api/        # REST API (axum), routes, entities, state
├── ironflow-engine/     # Workflow orchestration, FSM, handlers
├── ironflow-store/      # Storage trait + implementations (Postgres, in-memory)
├── ironflow-auth/       # JWT, password hashing, API keys
├── ironflow-worker/     # Background worker for run execution
├── ironflow-types/      # Shared API envelope types (ApiResponse, ErrorEnvelope)
├── ironflow-sdk/        # Type-safe Rust SDK (generated types from OpenAPI + hand-written client)
├── ironflow-cli/        # CLI tool (clap v4, consumes ironflow-sdk)
├── ironflow-mcp/        # MCP server (consumes ironflow-sdk)
```

## Build & Test Commands

```bash
cargo build                        # Build entire workspace
cargo test                         # Run all tests (unit + integration)
cargo test -p ironflow-core        # Core tests only
cargo test -p ironflow-runtime     # Runtime tests only
cargo doc --no-deps                # Build docs, check for warnings
cargo doc --no-deps --open         # Build and open in browser
```

## Documentation Rules

Every public item MUST have rustdoc documentation. No exceptions.

### Conventions

- `//!` for crate-level and module-level docs (top of file)
- `///` for structs, enums, traits, functions, methods, constants
- Write all documentation in **English**
- Use intra-doc links: `[`Type`]`, `[`Type::method`]`, `[`crate::module`]`
- Examples use `?` operator, never `unwrap()`

### Required Sections

| Section | When |
|---------|------|
| `# Examples` | Every public type and method (use `no_run` if it needs Claude CLI or network) |
| `# Errors` | Every function returning `Result` |
| `# Panics` | Every function with `assert!`, `panic!`, or `unwrap()` |

### Doc Test Patterns

```rust
/// ```no_run
/// use ironflow_core::prelude::*;
///
/// # async fn example() -> Result<(), OperationError> {
/// let shell = Shell::new("echo hello").await?;
/// # Ok(())
/// # }
/// ```
```

- Hide boilerplate with `# ` prefix (compiles but not rendered)
- Use `no_run` for anything requiring Claude CLI, network, or filesystem side effects
- Use `should_panic` for panic path examples

### Verification

`cargo doc --no-deps` must produce **zero warnings**. Check after every change.

## Testing Rules

### Principles

- **Black-box only** - NEVER create mocks. Test real behavior.
- Use fixture providers (like `RecordReplayProvider`) for agent tests - these replay real captured responses, not mocks.
- Shell tests spawn real processes.
- HTTP tests use real axum routers via `into_router()` + `tower::ServiceExt::oneshot`, or real TCP with `TcpListener::bind("127.0.0.1:0")`.

### Organization

| Location | Purpose |
|----------|---------|
| `#[cfg(test)] mod tests` inside source files | Unit tests (same module access) |
| `crate/tests/*.rs` | Integration tests (public API only) |
| Doc examples | Doc tests (compiled and run via `cargo test --doc`) |

### Test Coverage Targets

| Module | Target |
|--------|--------|
| `error.rs`, `utils.rs`, `tracker.rs` | 90%+ |
| `agent.rs` (builder + result), `shell.rs` | 80%+ |
| `webhook.rs` | 90%+ |
| `runtime.rs` | 70%+ (integration tests) |
| `providers/claude.rs` | Lower (real CLI tests marked `#[ignore]`) |
| `providers/record_replay.rs` | 90%+ |

### What to Test

For every new public function/method, add tests for:

1. **Happy path** - normal usage returns expected result
2. **Error paths** - invalid input, missing resources, timeouts
3. **Edge cases** - empty input, boundary values, unicode, very large input
4. **Panics** - `#[should_panic]` for assert-guarded inputs

### Async Test Pattern

```rust
#[tokio::test]
async fn test_name() {
    // For tests touching network/processes, wrap with timeout:
    tokio::time::timeout(Duration::from_secs(10), async {
        // test body
    }).await.expect("test timed out");
}
```

### HMAC Test Pattern

Use RFC 4231 test vectors for HMAC-SHA256 verification tests.

## Code Conventions

### Imports

**Zero tolerance for magic imports** -- always `use` imports, never inline qualified paths. Applies everywhere: function bodies, type annotations, return types, field types, static types, trait impls, and macro calls. No exceptions besides `crate::`/`self::`. Full rule with examples is auto-injected via `rust-imports.md` for all `.rs` files.

### Error Handling

- No retry logic - a step fails, the workflow fails
- Use `?` operator for propagation
- Return `OperationError` from operations, `AgentError` from providers

### Builder Pattern

Agent and Shell use the builder pattern. Validation happens at build time via `assert!` (panics on invalid input).

### Budget

Claude Code system cache costs ~$0.04. Set `max_budget_usd` to at least `0.10` in examples and tests to avoid budget-exceeded errors.

## SDK & CLI Propagation

When a change touches the API surface (new route, modified response, new field), it must propagate through the full chain:

1. **`ironflow-api`** -- add/modify the route and response entities
2. **`openapi.json`** -- regenerate the OpenAPI spec (`cargo test -p ironflow-api` produces it, or extract from `/api/v1/openapi.json`)
3. **`ironflow-sdk`** -- copy the updated `openapi.json` into `ironflow-sdk/openapi.json`. Types are auto-generated from it at build time via progenitor. If the new route needs an ergonomic client method, add it to `client.rs`.
4. **`ironflow-cli`** -- if the change adds a new command or modifies output, update the corresponding `commands/*.rs` and `output.rs` (table columns, colors, etc.)
5. **`ironflow-mcp`** -- if the change adds a new tool or modifies an existing one, update `ironflow-mcp/src/tools.rs`

**Rule:** a PR that adds or changes an API route without updating SDK + CLI is incomplete.

## When Adding New Features

Checklist for every PR:

1. Add rustdoc to all new public items (with `# Examples`, `# Errors`, `# Panics`)
2. Add unit tests in `#[cfg(test)] mod tests`
3. Add integration tests if the feature involves I/O
4. Run `cargo doc --no-deps` - zero warnings
5. Run `cargo test` - all tests pass
6. Update `plan.md` if a roadmap item is completed
7. Propagate API changes to SDK, CLI, and MCP (see **SDK & CLI Propagation** above)
