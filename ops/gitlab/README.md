# ironflow-ops-gitlab

GitLab integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, powered by the [`gitlab`](https://crates.io/crates/gitlab) crate.

Provides typed, builder-based access to every GitLab API v4 endpoint with automatic credential resolution from the workflow's secret store.

## Usage

```toml
[dependencies]
ironflow-ops-gitlab = "0.1"
```

### Build a client

```rust
use ironflow_ops_gitlab::GitLab;

// From a workflow step (reads gitlab_token from the secret store)
let gitlab = GitLab::from_context(&ctx).await?;

// Self-hosted instance
let gitlab = GitLab::from_context_with_host(&ctx, "gitlab.example.com").await?;

// Explicit token
let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
```

### Typed queries

Use the re-exported `gitlab::api` builders for type-safe endpoint calls:

```rust
use ironflow_ops_gitlab::GitLab;
use gitlab::api::{projects, AsyncQuery};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Project {
    id: u64,
    name: String,
}

let gitlab = GitLab::from_context(&ctx).await?;
let endpoint = projects::Project::builder().project(42).build()?;
let project: Project = endpoint.query_async(gitlab.client()).await?;
```

### Tracked operations

Wrap any endpoint in a tracked workflow step:

```rust
use ironflow_ops_gitlab::GitLab;
use gitlab::api::projects::issues::CreateIssue;

let gitlab = GitLab::from_context(&ctx).await?;

let endpoint = CreateIssue::builder()
    .project("my-group/my-project")
    .title("Bug report")
    .description("Steps to reproduce...")
    .build()?;

// Tracked as a workflow step with kind "gitlab"
let output = ctx.operation("create-issue", &gitlab.op(endpoint)).await?;
```

## Authentication

Register `gitlab_token` in your workflow's secret store:

```yaml
secrets:
  - name: gitlab_token
    env: GITLAB_TOKEN
```

## API coverage

This crate re-exports the full [`gitlab`](https://docs.rs/gitlab) API. All endpoints available in that crate are available here, including:

- Projects, groups, users
- Issues, merge requests, notes, discussions
- Pipelines, jobs, artifacts
- Repository files, branches, tags, commits
- Releases, deployments, environments
- CI/CD variables, runners, registry
- Labels, milestones, wikis, snippets, packages
- And more -- see the [gitlab crate docs](https://docs.rs/gitlab)
