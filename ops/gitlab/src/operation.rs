//! [`GitLabOp`] -- wraps any [`Endpoint`] as a tracked [`Operation`].

use async_trait::async_trait;
use gitlab::AsyncGitlab;
use gitlab::api::{AsyncQuery, Endpoint, raw};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use ironflow_ops_common::redact::Redactor;
use serde_json::{Map, Value};

use crate::masked::{Masked, mask_body};

/// A GitLab endpoint wrapped as an Ironflow [`Operation`].
///
/// Created via [`GitLab::op`](crate::GitLab::op). Implements `Operation` so it
/// can be passed to `WorkflowContext::operation()` for step lifecycle tracking
/// (step record, status transitions, duration, output persistence).
///
/// The endpoint is executed against the cloned [`AsyncGitlab`] client and the
/// response is deserialized as a JSON [`Value`].
///
/// Before sending, the request body is masked: every known secret of the
/// [`GitLab`](crate::GitLab) client and every token-shaped string
/// (`sk-ant-...`, `glpat-...`) is replaced with `[REDACTED]`. A form or JSON
/// body is masked field by field and a `text/*` body as text. Any other body
/// is file content (a generic package, a project import) and is sent as is.
/// A repository file sent in a form field (`CreateFile`, `CreateCommit` with
/// text encoding) is masked like any other field.
///
/// [`masks_secrets`](GitLabOp::masks_secrets) tells whether anything is
/// masked, and the step input records it as `"secrets_masked": true`.
/// [`without_masking`](GitLabOp::without_masking) sends a secret written on
/// purpose (a CI/CD variable) as is.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_core::operation::{Operation, OperationContext, NoopSecretResolver};
/// use gitlab::api::projects;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
/// let gitlab = GitLab::from_context(&ctx).await?;
///
/// let endpoint = projects::Project::builder().project(42).build()?;
/// let op = gitlab.op(endpoint);
///
/// assert_eq!(op.kind(), "gitlab");
/// # Ok(())
/// # }
/// ```
pub struct GitLabOp<E> {
    client: AsyncGitlab,
    endpoint: E,
    /// `None` once [`without_masking`](GitLabOp::without_masking) was called.
    redactor: Option<Redactor>,
}

impl<E> GitLabOp<E> {
    pub(crate) fn new(client: AsyncGitlab, endpoint: E, redactor: Redactor) -> Self {
        Self {
            client,
            endpoint,
            redactor: Some(redactor),
        }
    }

    /// Send the body as is, without masking secrets.
    ///
    /// For a request that writes a secret on purpose: a CI/CD variable, a
    /// webhook token, a runner token. Masking it would store `[REDACTED]` as
    /// its value.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    /// use gitlab::api::projects::variables::CreateProjectVariable;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let deploy_token = String::new();
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
    /// let endpoint = CreateProjectVariable::builder()
    ///     .project(42)
    ///     .key("DEPLOY_TOKEN")
    ///     .value(deploy_token)
    ///     .build()?;
    /// let op = gitlab.op(endpoint).without_masking();
    /// # Ok(())
    /// # }
    /// ```
    pub fn without_masking(mut self) -> Self {
        self.redactor = None;
        self
    }
}

impl<E: Endpoint> GitLabOp<E> {
    /// Whether the request body carries a secret that
    /// [`execute`](Operation::execute) masks before sending it.
    ///
    /// Computed from the endpoint, not from a past send: on a resumed run,
    /// where the engine replays the step instead of executing it again, the
    /// answer is the same as on the first execution.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    /// use ironflow_ops_gitlab::endpoints::merge_requests::CreateMergeRequestDiscussionNote;
    /// use gitlab::api::common::NameOrId;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
    /// let op = gitlab.op(CreateMergeRequestDiscussionNote {
    ///     project: NameOrId::from(42),
    ///     merge_request: 7,
    ///     discussion_id: "abcd1234".to_string(),
    ///     body: "agent output".to_string(),
    /// });
    /// if op.masks_secrets() {
    ///     eprintln!("a secret was masked in the note");
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn masks_secrets(&self) -> bool {
        let Some(redactor) = &self.redactor else {
            return false;
        };
        matches!(
            self.endpoint.body(),
            Ok(Some((content_type, body))) if mask_body(redactor, content_type, &body).is_some()
        )
    }
}

#[async_trait]
impl<E> Operation for GitLabOp<E>
where
    E: Endpoint + Sync + Send,
{
    fn kind(&self) -> &str {
        "gitlab"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        // Read the raw body: a `204 No Content` (a DELETE) has none, and the
        // JSON query would reject it.
        let masked = Masked {
            inner: &self.endpoint,
            redactor: self.redactor.as_ref(),
        };
        let body: Vec<u8> =
            raw(&masked)
                .query_async(&self.client)
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: e.to_string(),
                })?;
        if body.iter().all(u8::is_ascii_whitespace) {
            return Ok(Value::Null);
        }
        serde_json::from_slice(&body).map_err(|e| OperationError::Http {
            status: None,
            message: format!("could not parse the GitLab response: {e}"),
        })
    }

    fn input(&self) -> Option<Value> {
        let mut input = Map::from_iter([(
            "endpoint".to_string(),
            Value::String(self.endpoint.endpoint().into_owned()),
        )]);
        // Persisted on the step record: the run history shows that a secret
        // was masked, even when nobody called `masks_secrets`.
        if self.masks_secrets() {
            input.insert("secrets_masked".to_string(), Value::Bool(true));
        }
        Some(Value::Object(input))
    }
}
