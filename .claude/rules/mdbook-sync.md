# mdBook Documentation Sync

When a change touches any of the files below, check whether the mdBook
documentation in `docs/book/src/` needs updating.

## Trigger files

| File / directory | Doc pages to check |
|------------------|--------------------|
| `ironflow-engine/src/context.rs` | `concepts/steps.md`, `concepts/approval-gates.md`, `guides/writing-a-workflow.md`, `guides/parallel-execution.md` |
| `ironflow-engine/src/operation.rs` | `concepts/operations.md`, `guides/writing-an-operation.md` |
| `ironflow-engine/src/handler.rs` | `concepts/workflow-handler.md`, `guides/writing-a-workflow.md` |
| `ironflow-engine/src/config/` | `concepts/steps.md`, `guides/writing-a-workflow.md` |
| `ironflow-engine/src/engine.rs` | `concepts/engine-worker.md` |
| `ironflow-worker/` | `concepts/engine-worker.md`, `getting-started/worker.md` |
| `ironflow-api/` | `getting-started/server.md`, `architecture/overview.md` |
| `ironflow-core/src/providers/` | `guides/transports.md` |
| `examples/` | All pages using `{{#include}}` -- a renamed or deleted example breaks the build |

## What to check

1. **Signature changes**: if a public method signature changed (parameters,
   return type, name), update every inline `rust,ignore` block that shows it.
   Blocks using `{{#include}}` from `examples/` update automatically.
2. **New concepts**: if a new step kind, config type, or public trait was added,
   add it to the relevant concept page and update `SUMMARY.md` if a new page
   is warranted.
3. **Removed API**: if a public method or type was removed, grep
   `docs/book/src/` for its name and remove or replace every mention.
4. **New examples**: if a new example was added to `examples/`, consider
   including it in a guide page via `{{#include}}`.

## Build check

Run `mdbook build` after doc changes. A broken `{{#include}}` path fails the
build. This is also checked in CI on MR pipelines.
