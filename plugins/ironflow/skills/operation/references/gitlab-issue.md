# Example: create a GitLab issue

Uses `ironflow-ops-gitlab`, a thin wrapper around the [`gitlab`](https://crates.io/crates/gitlab)
crate. The token is read from the workflow secrets, the endpoint is built with the crate's
typed builders, and the result is a tracked workflow step.

Add the dependency:

```bash
cargo add -p workflows ironflow-ops-gitlab
```

## Typed query (no tracking)

When you only need the response and do not need step lifecycle tracking:

```rust,no_run
use gitlab::api::{projects::issues::CreateIssue, AsyncQuery};
use ironflow_ops_gitlab::GitLab;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Issue {
    iid: u64,
    web_url: String,
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let token = ctx
        .secrets()
        .get("gitlab_token")
        .await
        .map_err(EngineError::Store)?
        .ok_or_else(|| EngineError::StepConfig("secret gitlab_token missing".to_string()))?;

    let gitlab = GitLab::new(&token.value, "gitlab.com")
        .await
        .map_err(EngineError::Operation)?;

    let endpoint = CreateIssue::builder()
        .project("my-group/my-project")
        .title("Bug report")
        .description("Steps to reproduce...")
        .build()
        .map_err(|e| EngineError::StepConfig(e.to_string()))?;

    let issue: Issue = endpoint
        .query_async(gitlab.client())
        .await
        .map_err(|e| EngineError::StepConfig(e.to_string()))?;

    println!("Created issue !{} at {}", issue.iid, issue.web_url);
    Ok(())
}
```

## Tracked operation

Wrap the endpoint in `gitlab.op(endpoint)` to get step lifecycle tracking
(step record, status, duration, output persistence):

```rust,no_run
use gitlab::api::projects::issues::CreateIssue;
use ironflow_ops_gitlab::GitLab;
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;

async fn example(ctx: &mut WorkflowContext, failing_step: &str) -> Result<(), EngineError> {
    let token = ctx
        .secrets()
        .get("gitlab_token")
        .await
        .map_err(EngineError::Store)?
        .ok_or_else(|| EngineError::StepConfig("secret gitlab_token missing".to_string()))?;

    let gitlab = GitLab::new(&token.value, "gitlab.com")
        .await
        .map_err(EngineError::Operation)?;

    let labels = ["ci".to_string(), "automated".to_string()];
    let endpoint = CreateIssue::builder()
        .project("12345")
        .title(format!("Nightly build failed at step {failing_step}"))
        .description(format!("Run {} failed.", ctx.run_id()))
        .labels(labels.iter())
        .build()
        .map_err(|e| EngineError::StepConfig(e.to_string()))?;

    let issue = ctx.operation("open-issue", &gitlab.op(endpoint)).await?;

    let url = issue.output["web_url"].as_str().unwrap_or_default().to_string();
    ctx.shell(
        "announce",
        ShellConfig::new("echo \"Issue opened: $ISSUE_URL\"").env("ISSUE_URL", &url),
    )
    .await?;
    Ok(())
}
```

## Self-hosted instance

Pass your host instead of `"gitlab.com"`:

```rust,ignore
let gitlab = GitLab::new(&token.value, "gitlab.example.com")
    .await
    .map_err(EngineError::Operation)?;
```

Store the token once in the dashboard (Secrets, workflow scope) under the key `gitlab_token`.
