# Example: create a GitLab issue

Token read from the workflow secrets, project id and title from the handler, issue id
and url persisted as the step output.

```rust,no_run
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use ironflow_core::operations::http::Http;
use ironflow_core::error::OperationError;
use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;
use serde::Deserialize;
use serde_json::{Value, json};

/// `POST /projects/:id/issues` on a GitLab instance.
pub struct CreateGitlabIssue {
    /// Instance base url, for example `https://gitlab.com`.
    pub base_url: String,
    /// Numeric project id or url-encoded `group%2Fproject`.
    pub project: String,
    pub title: String,
    pub description: String,
    pub labels: Vec<String>,
    /// Personal or project access token with `api` scope.
    pub token: String,
}

#[derive(Debug, Deserialize)]
struct IssueCreated {
    iid: u64,
    web_url: String,
}

impl Operation for CreateGitlabIssue {
    fn kind(&self) -> &str {
        "gitlab"
    }

    fn input(&self) -> Option<Value> {
        Some(json!({
            "project": self.project,
            "title": self.title,
            "labels": self.labels,
        }))
    }

    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
        Box::pin(async move {
            let url = format!("{}/api/v4/projects/{}/issues", self.base_url, self.project);
            let resp = Http::post(&url)
                .header("PRIVATE-TOKEN", &self.token)
                .json(json!({
                    "title": self.title,
                    "description": self.description,
                    "labels": self.labels.join(","),
                }))
                .timeout(Duration::from_secs(30))
                .await?;

            if !resp.is_success() {
                return Err(EngineError::Operation(OperationError::Http {
                    status: Some(resp.status()),
                    message: format!("gitlab answered {}: {}", resp.status(), resp.body()),
                }));
            }

            let issue: IssueCreated = resp.json()?;
            Ok(json!({"iid": issue.iid, "url": issue.web_url}))
        })
    }
}
```

Handler side, with the token from the secret store and the issue url reused later:

```rust,no_run
use ironflow_engine::config::ShellConfig;
use ironflow_engine::context::WorkflowContext;
use ironflow_engine::error::EngineError;
use ironflow_engine::operation::Operation;
use serde_json::{Value, json};
use std::future::Future;
use std::pin::Pin;

struct CreateGitlabIssue {
    base_url: String,
    project: String,
    title: String,
    description: String,
    labels: Vec<String>,
    token: String,
}

impl Operation for CreateGitlabIssue {
    fn kind(&self) -> &str {
        "gitlab"
    }
    fn execute(&self) -> Pin<Box<dyn Future<Output = Result<Value, EngineError>> + Send + '_>> {
        Box::pin(async move { Ok(json!({"iid": 42, "url": "https://gitlab.com/x/y/-/issues/42"})) })
    }
}

async fn example(ctx: &mut WorkflowContext, failing_step: &str) -> Result<(), EngineError> {
    let token = ctx
        .secrets()
        .get("gitlab_token")
        .await
        .map_err(EngineError::Store)?
        .ok_or_else(|| EngineError::StepConfig("secret gitlab_token missing".to_string()))?
        .value;

    let op = CreateGitlabIssue {
        base_url: "https://gitlab.com".to_string(),
        project: "12345".to_string(),
        title: format!("Nightly build failed at step {failing_step}"),
        description: format!("Run {} failed.", ctx.run_id()),
        labels: vec!["ci".to_string(), "automated".to_string()],
        token,
    };
    let issue = ctx.operation("open-issue", &op).await?;

    let url = issue.output["url"].as_str().unwrap_or_default().to_string();
    ctx.shell(
        "announce",
        ShellConfig::new("echo \"Issue opened: $ISSUE_URL\"").env("ISSUE_URL", &url),
    )
    .await?;
    Ok(())
}
```

Store the token once in the dashboard (Secrets, workflow scope) under the key `gitlab_token`.
