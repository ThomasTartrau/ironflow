# Parallel Execution

Ironflow supports running multiple steps in parallel within a workflow.

## Using ctx.parallel()

Pass a list of step configurations to `ctx.parallel()`. All steps run concurrently and the method returns when all complete:

```rust,ignore
{{#include ../../../../examples/ironflow-workflows/src/ci_pipeline.rs}}
```

## How it works

- All steps in a `parallel()` call start at the same time
- The method returns a `Vec<ParallelResult>` with outputs in the same order as the input
- If `fail_fast` is `true` (the second argument), the remaining steps are cancelled when one fails
- If `fail_fast` is `false`, all steps run to completion regardless of individual failures

## Conditional branching

Since workflows are Rust code, conditional logic is just `if`/`else`:

```rust,ignore
let results = ctx.parallel(steps, true).await?;
let all_passed = results.iter().all(|r| r.output.is_success());

if all_passed {
    ctx.shell("deploy", ShellConfig::new("echo 'Deploying'")).await?;
} else {
    ctx.shell("notify", ShellConfig::new("echo 'Tests failed'")).await?;
}
```

No special DSL for branching -- Rust control flow works directly.
