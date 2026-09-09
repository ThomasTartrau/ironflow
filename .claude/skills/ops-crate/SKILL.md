---
name: ops-crate
description: Generate a new ops/xxx crate for an external service integration. Identifies the right pattern (A=wrapper, B=HTTP direct, C=macro), scaffolds the full structure, and runs the quality checklist. Trigger on "ops crate", "new ops", "creer un crate ops", "add ops/xxx", "nouveau crate ops", "ironflow ops".
argument-hint: "<service-name> [pattern-a|pattern-b|pattern-c]"
---

# ops-crate -- Generate a new Ironflow ops crate

## Arguments

`$ARGUMENTS` -- expected: `<service-name>` and optionally a pattern override.

Parse the arguments:
- First word: the service name (e.g. `datadog`, `pagerduty`, `cloudflare`)
- Second word (optional): `pattern-a`, `pattern-b`, or `pattern-c`

If no service name is given, ask for one.

## Step 1: Check if the crate already exists

```bash
test -d "ops/$SERVICE" && echo "EXISTS" || echo "NEW"
```

If EXISTS: warn the user and ask whether to continue (extend) or abort.

## Step 2: Identify the pattern

Read the reference files to understand each pattern:
- `references/pattern-a-wrapper.md` -- Pattern A
- `references/pattern-b-http.md` -- Pattern B
- `references/pattern-c-macro.md` -- Pattern C
- `references/common-helpers.md` -- Shared helpers

### If a pattern was specified in arguments

Use it directly.

### If no pattern was specified

Ask the user with AskUserQuestion:

| Criterion | Pattern A (Wrapper) | Pattern B (HTTP direct) | Pattern C (Macro) |
|-----------|---------------------|-------------------------|-------------------|
| Rust crate exists on crates.io? | Yes, with uniform trait | No | Yes, with typed methods 1:1 |
| HTTP handled by | Underlying crate | ironflow-ops-common's HttpApiClient | Underlying crate |
| Operation granularity | Generic `Op<E>` for all endpoints | One struct per endpoint | Macro-generated per method |
| Boilerplate | Minimal | Medium (one struct per operation) | Minimal (macro generates) |
| Reference crates | ops/gitlab, ops/k8s | ops/grafana, ops/loki, ops/tempo, ops/mimir | ops/slack |

Recommend Pattern B if unsure -- it is the most common and self-contained.

## Step 3: Scaffold the crate

Follow the structure from the chosen pattern's reference file.

### 3a. Create directory

```bash
mkdir -p ops/$SERVICE/src
```

### 3b. Create Cargo.toml

Use `cargo add` after creating the basic file. The template is in
`references/common-helpers.md` (Pattern B) or adapt for A/C.

**Pattern B dependencies:**
- `ironflow-core` (path = "../../ironflow-core")
- `ironflow-ops-common` (path = "../common")
- `async-trait`
- `reqwest` with features ["json", "query"]
- `serde` with features ["derive"]
- `serde_json`
- Dev: `tokio` with features ["full"], `wiremock`

**Pattern A dependencies:**
- `ironflow-core` (path = "../../ironflow-core")
- `async-trait`
- The underlying Rust crate
- `serde_json`
- Dev: `tokio` with features ["full"]

**Pattern C dependencies:**
- `ironflow-core` (path = "../../ironflow-core")
- `async-trait`
- The underlying Rust crate
- `serde`, `serde_json`
- Dev: `tokio` with features ["full"]

### 3c. Create source files

Follow the exact structure from the reference file for the chosen pattern.

**Pattern B files:**
1. `src/lib.rs` -- crate-level doc, pub modules, `pub use client::XxxClient`
2. `src/client.rs` -- XxxClient wrapping HttpApiClient (copy structure from reference)
3. One example module with 1-2 operations to demonstrate the pattern
4. `tests/` directory with wiremock integration tests

**Pattern A files:**
1. `src/lib.rs` -- crate-level doc, re-exports, `pub use underlying_crate`
2. `src/client.rs` -- XxxClient with from_context + op()
3. `src/operation.rs` -- XxxOp<E> generic Operation wrapper

**Pattern C files:**
1. `src/lib.rs` -- crate-level doc, re-exports
2. `src/client.rs` -- XxxClient with from_context, session()
3. `src/error.rs` -- error conversion
4. `src/macros.rs` -- xxx_op! macro
5. One example module using the macro

### 3d. Register in workspace

Add `"ops/$SERVICE"` to the `members` list in the root `Cargo.toml`.

## Step 4: Quality checklist

Run these commands and fix any issues:

```bash
cargo clippy --all-targets -p ironflow-ops-$SERVICE
cargo doc --no-deps -p ironflow-ops-$SERVICE
cargo test -p ironflow-ops-$SERVICE
```

### Verify no duplication with ops/common

Check that the new crate does NOT re-implement any of these helpers:
- `check_response` / `check_response_json`
- `validate_path_segment`
- `send_request`
- `parse_json_body`
- `to_value` (Pattern B must use `ironflow_ops_common::helpers::to_value`)
- `reqwest_err`

Exception: Pattern C's `macros.rs` has its own `to_value` with a
service-specific error origin -- this is intentional.

### Mandatory tests (all patterns)

Every ops crate MUST include at minimum:

**Client tests:**
- `from_context_fails_when_token_missing` -- fast, no network
- `new` trims trailing slash on base URL (Pattern B only)
- `new` rejects empty token
- `debug_does_not_leak_token` -- format with Debug, assert token absent

**Operation tests (Pattern B with wiremock):**
- URL constructed correctly (mock matches on `path`)
- Auth header sent (mock matches on `bearer_token`)
- Body sent correctly (POST/PUT/PATCH)
- 4xx/5xx returns OperationError::Http
- `input()` does NOT contain secrets
- `kind()` returns the correct service name

**Operation tests (Pattern A -- mostly #[ignore]):**
- `kind()` returns the correct service name
- `input()` returns endpoint metadata

**Operation tests (Pattern C):**
- `to_value` serializes a struct
- `to_value` serializes unit to null

### Documentation requirements

Every public item needs rustdoc:
- `//!` for crate-level and module-level docs
- `///` for structs, enums, traits, functions, methods
- `# Examples` with `no_run` for anything needing network
- `# Errors` for every `Result`-returning function

## Step 5: Suggest next steps

After generation, tell the user:

1. Add the actual operations for the target API
2. Run the full test suite: `cargo test -p ironflow-ops-$SERVICE`
3. Consider adding the crate to the `ironflow-ops` meta-crate if one exists

## Conventions

- All code follows the project's CLAUDE.md rules (imports at top, no inline paths)
- Use `?` operator for error propagation, return `OperationError` variants
- `Debug` implementations MUST redact secrets
- `input()` implementations MUST NOT include secrets
- Crate names follow the pattern `ironflow-ops-$SERVICE`
- Package names use hyphens: `ironflow-ops-xxx`
- Module names use underscores in Rust: `ironflow_ops_xxx`
