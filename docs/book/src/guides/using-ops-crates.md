# Using a Pre-built Ops Crate

Ironflow ships with 13 ops crates under `ops/` that provide ready-to-use integrations for common services. Instead of implementing the [`Operation`](../concepts/operations.md) trait yourself, you can use these crates to get typed, tracked operations in a single `cargo add`.

## The pattern

Every ops crate follows the same three-step pattern:

1. **Add the dependency** to your workflow crate
2. **Build a client** from your workflow's `OperationContext` (via `from_context()`)
3. **Run a tracked operation** via `ctx.operation()`

```rust,ignore
use ironflow_ops_slack::SlackClient;
use ironflow_ops_slack::chat::ChatPostMessage;
use slack_morphism::api::SlackApiChatPostMessageRequest;
use slack_morphism::{SlackChannelId, SlackMessageContent};

// 1. Build the client (reads slack_bot_token from the secret store)
let slack = SlackClient::from_context(&ctx).await?;

// 2. Create the operation
let req = SlackApiChatPostMessageRequest::new(
    SlackChannelId::new("#deployments".to_string()),
    SlackMessageContent::new().with_text("Deploy complete".to_string()),
);
let op = ChatPostMessage::new(&slack, req);

// 3. Execute as a tracked workflow step
let output = ctx.operation("notify-team", &op).await?;
```

The step is tracked in the database with its input, output, kind, and status, just like a shell or HTTP step.

## Available crates

| Crate | Description | `cargo add` |
|-------|-------------|-------------|
| [`ironflow-ops-common`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/common) | Shared HTTP client and helpers for ops crates (not used directly in workflows) | `cargo add ironflow-ops-common` |
| [`ironflow-ops-docker`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/docker) | Docker containers, images, networks, volumes via bollard | `cargo add ironflow-ops-docker` |
| [`ironflow-ops-git`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/git) | Git operations (commit, branch, merge, diff, ...) via git2 | `cargo add ironflow-ops-git` |
| [`ironflow-ops-gitlab`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/gitlab) | GitLab API v4 (issues, MRs, pipelines, ...) via the gitlab crate | `cargo add ironflow-ops-gitlab` |
| [`ironflow-ops-grafana`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/grafana) | Grafana API (dashboards, alerting, data sources, ...) | `cargo add ironflow-ops-grafana` |
| [`ironflow-ops-helm`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/helm) | Helm CLI wrapper (install, upgrade, rollback, charts, repos) | `cargo add ironflow-ops-helm` |
| [`ironflow-ops-k8s`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/k8s) | Kubernetes typed API via kube + k8s-openapi | `cargo add ironflow-ops-k8s` |
| [`ironflow-ops-loki`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/loki) | Grafana Loki (log queries, ingest, rules, labels) | `cargo add ironflow-ops-loki` |
| [`ironflow-ops-mimir`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/mimir) | Grafana Mimir (PromQL queries, remote write, rules, cardinality) | `cargo add ironflow-ops-mimir` |
| [`ironflow-ops-postgres`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/postgres) | PostgreSQL queries and admin via sqlx | `cargo add ironflow-ops-postgres` |
| [`ironflow-ops-s3`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/s3) | AWS S3 objects, buckets, presigned URLs via aws-sdk-s3 | `cargo add ironflow-ops-s3` |
| [`ironflow-ops-slack`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/slack) | Slack API (chat, conversations, files, users) via slack-morphism | `cargo add ironflow-ops-slack` |
| [`ironflow-ops-tempo`](https://gitlab.com/ThomasTartrau/ironflow/-/tree/main/ops/tempo) | Grafana Tempo (trace queries, search, metrics, cluster) | `cargo add ironflow-ops-tempo` |

## Example: GitLab integration

```rust,ignore
use ironflow_ops_gitlab::GitLab;
use gitlab::api::projects::issues::CreateIssue;

// Build from workflow context (reads gitlab_token from the secret store)
let gitlab = GitLab::from_context(&ctx).await?;

// For a self-hosted instance
let gitlab = GitLab::from_context_with_host(&ctx, "gitlab.example.com").await?;

// Create an issue as a tracked step
let endpoint = CreateIssue::builder()
    .project("my-group/my-project")
    .title("Automated bug report")
    .description("Detected by workflow")
    .build()?;

let output = ctx.operation("create-issue", &gitlab.op(endpoint)).await?;
```

## Example: Kubernetes + Helm

```rust,ignore
use ironflow_ops_k8s::{KubeClient, verb};
use ironflow_ops_helm::HelmClient;
use ironflow_ops_helm::release::Upgrade;
use k8s_openapi::api::apps::v1::Deployment;

// Check deployment status
let kube = KubeClient::from_context(&ctx).await?;
let deployments = kube.namespaced::<Deployment>("production");
let op = kube.op(deployments, verb::Get::new("my-app"));
ctx.operation("check-deployment", &op).await?;

// Upgrade the Helm release
let helm = HelmClient::from_context(&ctx).await?;
let upgrade = Upgrade::new(helm, "my-app", "charts/my-app")
    .namespace("production")
    .set("image.tag", "v2.1.0");
ctx.operation("upgrade-release", &upgrade).await?;
```

## Secrets and authentication

Each crate documents its required and optional secrets in its README. The general pattern is:

1. Register secrets in your workflow's secret store
2. Call `from_context(&ctx)` which resolves them automatically
3. The client handles authentication transparently

See each crate's README for the exact secret names and formats.

## Writing your own operation

If none of the pre-built crates cover your service, see [Writing an Operation](writing-an-operation.md) to implement the `Operation` trait directly.
