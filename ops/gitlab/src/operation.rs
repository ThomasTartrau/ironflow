//! [`GitLabOp`] -- wraps any [`Endpoint`] as a tracked [`Operation`].

use async_trait::async_trait;
use gitlab::AsyncGitlab;
use gitlab::api::{AsyncQuery, Endpoint, raw};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

/// A GitLab endpoint wrapped as an Ironflow [`Operation`].
///
/// Created via [`GitLab::op`](crate::GitLab::op). Implements `Operation` so it
/// can be passed to `WorkflowContext::operation()` for step lifecycle tracking
/// (step record, status transitions, duration, output persistence).
///
/// The endpoint is executed against the cloned [`AsyncGitlab`] client and the
/// response is deserialized as a JSON [`Value`].
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
}

impl<E> GitLabOp<E> {
    pub(crate) fn new(client: AsyncGitlab, endpoint: E) -> Self {
        Self { client, endpoint }
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
        let body: Vec<u8> = raw(&self.endpoint)
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
        Some(Value::Object(serde_json::Map::from_iter([(
            "endpoint".to_string(),
            Value::String(self.endpoint.endpoint().into_owned()),
        )])))
    }
}
