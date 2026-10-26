# Example: create a GitLab issue

Uses `ironflow-ops-gitlab`, a thin wrapper around the [`gitlab`](https://crates.io/crates/gitlab)
crate. The token is read from the workflow secrets, the endpoint is built with the crate's
typed builders, and the result is a tracked workflow step.

Add the dependency:

```bash
cargo add -p workflows ironflow-ops-gitlab
```

## Typed query (no tracking)

When you only need to read a response and do not need step lifecycle tracking. A write
sent this way is not masked (see "Secrets in the published text"): publish through
`gitlab.op`.

```rust,no_run
use gitlab::api::{projects::Project, AsyncQuery};
use ironflow_ops_gitlab::GitLab;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct ProjectInfo {
    id: u64,
    web_url: String,
}

async fn example(ctx: &mut WorkflowContext) -> Result<(), EngineError> {
    let token = ctx
        .secrets()?
        .get("gitlab_token")
        .await
        .map_err(EngineError::Store)?
        .ok_or_else(|| EngineError::StepConfig("secret gitlab_token missing".to_string()))?;

    let gitlab = GitLab::new(&token.value, "gitlab.com")
        .await
        .map_err(EngineError::Operation)?;

    let endpoint = Project::builder()
        .project("my-group/my-project")
        .build()
        .map_err(|e| EngineError::StepConfig(e.to_string()))?;

    let project: ProjectInfo = endpoint
        .query_async(gitlab.client())
        .await
        .map_err(|e| EngineError::StepConfig(e.to_string()))?;

    println!("Project {} at {}", project.id, project.web_url);
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
use serde::Deserialize;

/// The fields of the created issue the handler needs.
#[derive(Deserialize)]
struct Issue {
    web_url: String,
}

async fn example(ctx: &mut WorkflowContext, failing_step: &str) -> Result<(), EngineError> {
    let token = ctx
        .secrets()?
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

    let issue: Issue = ctx.operation("open-issue", &gitlab.op(endpoint)).await?.json()?;

    ctx.shell(
        "announce",
        ShellConfig::new("echo \"Issue opened: $ISSUE_URL\"").env("ISSUE_URL", &issue.web_url),
    )
    .await?;
    Ok(())
}
```

## Secrets in the published text

`gitlab.op(endpoint)` masks secrets in the request body: the client's token, the values
added with `GitLab::with_secrets([..])`, and `sk-ant-` / `glpat-` tokens become
`[REDACTED]`. A query sent through `gitlab.client()` is not masked: publish agent or command
output through `gitlab.op`. `op.masks_secrets()` tells whether anything is masked. A request
that writes a secret on purpose (a CI/CD variable) goes through `gitlab.op(endpoint).without_masking()`.

## Self-hosted instance

Pass your host instead of `"gitlab.com"`:

```rust,ignore
let gitlab = GitLab::new(&token.value, "gitlab.example.com")
    .await
    .map_err(EngineError::Operation)?;
```

Store the token once in the dashboard (Secrets, workflow scope) under the key `gitlab_token`.
