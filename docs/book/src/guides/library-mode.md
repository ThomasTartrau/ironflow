# Library Mode

`ironflow-core` works on its own, without the API server, a worker or a database. Add it to
any binary and call the operations directly: shell commands, HTTP requests and AI agents,
with timeouts, budgets and cost tracking. Nothing is persisted and there is no dashboard;
for that, use the full platform described in the rest of this book.

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

## Shell

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

`Shell::new` hands its string to `sh -c`. When the command carries data you do not control,
use `Shell::exec(program, &[args])`, which passes each argument as is.

## Http

A non-2xx status is not an error: check `is_success()`.

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

## Agent

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

Other providers (OpenAI, Gemini, Mistral, remote Claude Code) are listed in
[Agent Providers](agent-providers.md).

### Session resume

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

## Parallel execution

`tokio::try_join!` when the number of operations is known at compile time:

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

`try_join_all` when it is decided at run time, `try_join_all_limited` to cap concurrency:

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

## Tracking cost and duration

`WorkflowTracker` adds up cost, tokens and duration across operations:

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

## Dry-run mode

```rust,no_run
use ironflow_core::prelude::*;

# async fn example() -> Result<(), OperationError> {
// Global: every operation skips execution
set_dry_run(true);
let output = Shell::new("rm -rf /").await?; // not executed
assert_eq!(output.stdout(), "");

// Per operation, overrides the global setting
set_dry_run(false);
let output = Shell::new("echo hello").dry_run(true).await?;
assert_eq!(output.stdout(), "");
# Ok(())
# }
```

## Record/replay testing

`RecordReplayProvider` wraps any provider and stores its responses as JSON fixtures, keyed by
a hash of the prompt, system prompt and schema. Tests then replay them without spending
tokens.

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

To test a full workflow handler, see [Testing Workflows](testing-workflows.md).
