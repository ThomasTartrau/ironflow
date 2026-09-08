//! [`GitLab`] client built from an [`OperationContext`]'s secret store.

use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;

use crate::operation::GitLabOp;

/// A GitLab client that resolves credentials from the workflow's secret store.
///
/// Wraps [`AsyncGitlab`] and provides a convenience [`op`](GitLab::op) method
/// to turn any endpoint into a tracked [`Operation`](ironflow_core::operation::Operation).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_core::operation::{OperationContext, NoopSecretResolver};
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
///
/// // gitlab.com (default)
/// let gitlab = GitLab::from_context(&ctx).await?;
///
/// // Self-hosted
/// let gitlab = GitLab::from_context_with_host(&ctx, "gitlab.example.com").await?;
/// # Ok(())
/// # }
/// ```
pub struct GitLab {
    inner: AsyncGitlab,
}

impl GitLab {
    /// Build a client from an [`OperationContext`], defaulting to `gitlab.com`.
    ///
    /// Reads the `gitlab_token` secret from the workflow's secret store.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the token is missing, or
    /// [`OperationError::Http`] if the client cannot be built.
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        Self::from_context_with_host(ctx, "gitlab.com").await
    }

    /// Build a client from an [`OperationContext`] with a custom host.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Secret`] if the token is missing, or
    /// [`OperationError::Http`] if the client cannot be built.
    pub async fn from_context_with_host(
        ctx: &OperationContext,
        host: &str,
    ) -> Result<Self, OperationError> {
        let secret =
            ctx.secrets()
                .get("gitlab_token")
                .await?
                .ok_or_else(|| OperationError::Secret {
                    message: "gitlab_token secret not found".to_string(),
                })?;
        Self::new(&secret.value, host).await
    }

    /// Build a client with an explicit token and host.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Http`] if the client cannot be built.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn new(token: &str, host: &str) -> Result<Self, OperationError> {
        let inner = GitlabBuilder::new(host, token)
            .build_async()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: e.to_string(),
            })?;
        Ok(Self { inner })
    }

    /// The underlying [`AsyncGitlab`] client.
    ///
    /// Use this with [`AsyncQuery::query_async`](gitlab::api::AsyncQuery::query_async)
    /// for typed endpoint calls.
    pub fn client(&self) -> &AsyncGitlab {
        &self.inner
    }

    /// Wrap an endpoint as a tracked [`Operation`](ironflow_core::operation::Operation).
    ///
    /// The returned [`GitLabOp`] implements `Operation` so it can be passed
    /// to `WorkflowContext::operation()` for step lifecycle tracking.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    /// use gitlab::api::projects;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
    /// let endpoint = projects::Project::builder().project(42).build().unwrap();
    /// let op = gitlab.op(endpoint);
    /// # Ok(())
    /// # }
    /// ```
    pub fn op<E>(&self, endpoint: E) -> GitLabOp<E> {
        GitLabOp::new(self.inner.clone(), endpoint)
    }
}

impl std::fmt::Debug for GitLab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitLab")
            .field("client", &"[AsyncGitlab]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    use super::*;

    #[tokio::test]
    #[ignore]
    async fn new_builds_with_valid_host() {
        let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await;
        assert!(gitlab.is_ok());
    }

    #[tokio::test]
    #[ignore]
    async fn debug_does_not_leak_token() {
        let gitlab = GitLab::new("super-secret", "gitlab.com").await.unwrap();
        let debug = format!("{gitlab:?}");
        assert!(!debug.contains("super-secret"));
    }

    #[tokio::test]
    async fn from_context_fails_when_token_missing() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = GitLab::from_context(&ctx).await.unwrap_err();
        assert!(err.to_string().contains("gitlab_token"));
    }
}
