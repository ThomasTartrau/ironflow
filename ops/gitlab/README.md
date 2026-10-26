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

```rust
use ironflow_ops_gitlab::GitLab;
use gitlab::GitlabBuilder;

// From an already configured AsyncGitlab (e.g. wiremock in tests, custom TLS)
let client = GitlabBuilder::new("gitlab.example.com", "glpat-xxxx")
    .insecure()
    .build_async()
    .await?;
// The token of an AsyncGitlab cannot be read back: pass it to mask it.
let gitlab = GitLab::from(client).with_secrets(["glpat-xxxx"]);
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

### Secret masking

`GitLab::op` masks secrets in the request body before sending it, so text written by an
agent or a command never publishes a token in a note, an issue or a merge request. It masks
the client's own token, the values added with `with_secrets`, and token-shaped strings
(`sk-ant-...`, `glpat-...`), each replaced with `[REDACTED]`:

```rust
use tracing::warn;

let gitlab = GitLab::from_context(&ctx).await?.with_secrets([worker_token]);

let op = gitlab.op(endpoint);
if op.masks_secrets() {
    warn!("secret masked in the published issue");
}
ctx.operation("create-issue", &op).await?;
```

The step input records a masked request as `"secrets_masked": true`. Form, JSON and
`text/*` bodies are masked, a repository file sent as a form field included; other bodies
(a generic package, a project import) are sent as is. A request that writes a secret on
purpose (a CI/CD variable, a webhook token) goes through `gitlab.op(endpoint).without_masking()`.
A client built with `GitLab::from(AsyncGitlab)` does not know its token: add it with
`with_secrets`. A query sent through `gitlab.client()` is not masked.

### Endpoints not in the `gitlab` crate

Some GitLab REST routes are missing from the `gitlab` crate. This crate provides them
under `endpoints`, wired the same way as any other endpoint:

| Endpoint | Route |
|---|---|
| `merge_requests::CreateMergeRequestDiscussionNote` | `POST /projects/:id/merge_requests/:iid/discussions/:discussion_id/notes` |
| `merge_requests::ResolveMergeRequestDiscussion` | `PUT /projects/:id/merge_requests/:iid/discussions/:discussion_id` |
| `merge_requests::MergeRequestVersions` | `GET /projects/:id/merge_requests/:iid/versions` |
| `merge_requests::MergeRequestVersion` | `GET /projects/:id/merge_requests/:iid/versions/:version_id` |
| `merge_requests::DeleteMergeRequestNote` | `DELETE /projects/:id/merge_requests/:iid/notes/:note_id` |
| `repository::MergeBase` | `GET /projects/:id/repository/merge_base` |
| `discussions::Discussions` | `GET /projects/:id/issues/:iid/discussions`, `GET /projects/:id/repository/commits/:sha/discussions` |
| `discussions::Discussion` | `GET .../discussions/:discussion_id` (issue or commit) |
| `discussions::CreateDiscussion` | `POST .../discussions` (issue or commit) |
| `discussions::ReplyToDiscussion` | `POST .../discussions/:discussion_id/notes` (issue or commit) |
| `notes::Note` | `GET /projects/:id/issues/:iid/notes/:note_id`, `GET /projects/:id/merge_requests/:iid/notes/:note_id` |
| `releases::Releases` | `GET /projects/:id/releases` (one page of `per_page` items) |
| `uploads::UploadFile` | `POST /projects/:id/uploads` (multipart) |

A `204 No Content` answer (a deletion) comes back from `GitLab::op` as `Value::Null`.

```rust
use ironflow_ops_gitlab::GitLab;
use ironflow_ops_gitlab::endpoints::merge_requests::{
    CreateMergeRequestDiscussionNote, ResolveMergeRequestDiscussion,
};
use gitlab::api::common::NameOrId;

let gitlab = GitLab::from_context(&ctx).await?;

let note = CreateMergeRequestDiscussionNote {
    project: NameOrId::from(42),
    merge_request: 7,
    discussion_id: "abcd1234".to_string(),
    body: "Looks good, thanks!".to_string(),
};
let output = ctx.operation("reply-to-discussion", &gitlab.op(note)).await?;

let resolve = ResolveMergeRequestDiscussion {
    project: NameOrId::from(42),
    merge_request: 7,
    discussion_id: "abcd1234".to_string(),
    resolved: true,
};
let output = ctx.operation("resolve-discussion", &gitlab.op(resolve)).await?;
```

An upload answers with GitLab's JSON (`alt`, `url`, `full_path`, `markdown`). The file name is
sanitized, and `UploadFile::new` refuses a content that contains the multipart boundary marker:

```rust
use ironflow_ops_gitlab::GitLab;
use ironflow_ops_gitlab::endpoints::uploads::UploadFile;
use gitlab::api::common::NameOrId;
use serde::Deserialize;

let gitlab = GitLab::from_context(&ctx).await?;

let upload = UploadFile::new(NameOrId::from(42), "screenshot.png", png_bytes)?;
let output = ctx.operation("upload-screenshot", &gitlab.op(upload)).await?;

#[derive(Deserialize)]
struct Uploaded {
    markdown: String,
}
let uploaded: Uploaded = output.json()?;
// uploaded.markdown goes in the body of a note
```

### Paginated operations

`GitLab::op` issues a single request, so it only ever returns the first page of a "list ..."
endpoint. Use `GitLab::paged_op` to drive pagination and get back a JSON array concatenating
every page:

```rust
use ironflow_ops_gitlab::GitLab;
use gitlab::api::projects::merge_requests::MergeRequests;
use gitlab::api::Pagination;

let gitlab = GitLab::from_context(&ctx).await?;

let endpoint = MergeRequests::builder().project(42).build()?;
let output = ctx
    .operation("list-merge-requests", &gitlab.paged_op(endpoint, Pagination::All))
    .await?;
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

## Errors

A failed operation returns `OperationError::Http`. When GitLab answered with an error, `status` carries the HTTP code, so a conflict is detected with `OperationError::Http { status: Some(409), .. }` (for example a merge request that already exists) instead of parsing the message. `status` is `None` for transport and local errors.
