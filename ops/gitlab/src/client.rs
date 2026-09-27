//! [`GitLab`] client built from an [`OperationContext`]'s secret store.

use gitlab::api::{Endpoint, Pageable, Pagination};
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;

use crate::operation::GitLabOp;
use crate::paged_operation::GitLabPagedOp;

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

    /// Wrap a pageable endpoint as a tracked [`Operation`](ironflow_core::operation::Operation)
    /// that fetches every page requested by `pagination` and concatenates the results.
    ///
    /// Unlike [`GitLab::op`], this accepts endpoints that implement
    /// [`Pageable`](gitlab::api::Pageable) (e.g. any "list ..." endpoint) and drives
    /// pagination itself, so a single tracked step can retrieve more than one page
    /// (`GitLabOp` only ever issues a single request and therefore only ever returns
    /// the first page).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    /// use gitlab::api::projects::merge_requests::MergeRequests;
    /// use gitlab::api::Pagination;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
    /// let endpoint = MergeRequests::builder().project(42).build().unwrap();
    /// let op = gitlab.paged_op(endpoint, Pagination::All);
    /// # Ok(())
    /// # }
    /// ```
    pub fn paged_op<E>(&self, endpoint: E, pagination: Pagination) -> GitLabPagedOp<E>
    where
        E: Pageable + Endpoint + Send + Sync,
    {
        GitLabPagedOp::new(self.inner.clone(), endpoint, pagination)
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

    use gitlab::api::projects::merge_requests::MergeRequests;
    use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
    use serde_json::{Value, json};
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    fn ctx() -> OperationContext {
        OperationContext::new(Arc::new(NoopSecretResolver))
    }

    async fn insecure_client(server: &MockServer) -> AsyncGitlab {
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1})))
            .mount(server)
            .await;

        GitlabBuilder::new(server.address().to_string(), "token")
            .insecure()
            .build_async()
            .await
            .unwrap()
    }

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

    #[tokio::test]
    async fn paged_op_all_pagination_concatenates_every_page() {
        let server = MockServer::start().await;
        let page1: Vec<Value> = (0..100).map(|i| json!({"iid": i})).collect();
        let page2: Vec<Value> = vec![
            json!({"iid": 100}),
            json!({"iid": 101}),
            json!({"iid": 102}),
        ];

        Mock::given(method("GET"))
            .and(path("/api/v4/projects/42/merge_requests"))
            .and(query_param("page", "1"))
            .and(query_param("per_page", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page1))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/42/merge_requests"))
            .and(query_param("page", "2"))
            .and(query_param("per_page", "100"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page2))
            .mount(&server)
            .await;

        let client = insecure_client(&server).await;
        let gitlab = GitLab { inner: client };
        let endpoint = MergeRequests::builder().project(42).build().unwrap();
        let op = gitlab.paged_op(endpoint, Pagination::All);

        let result = op.execute(&ctx()).await.unwrap();
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 103);
        assert_eq!(items[0]["iid"], 0);
        assert_eq!(items[100]["iid"], 100);
        assert_eq!(items[102]["iid"], 102);
    }

    #[tokio::test]
    async fn paged_op_limit_truncates_output() {
        let server = MockServer::start().await;
        let page: Vec<Value> = vec![json!({"iid": 0})];

        Mock::given(method("GET"))
            .and(path("/api/v4/projects/42/merge_requests"))
            .and(query_param("page", "1"))
            .and(query_param("per_page", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(&page))
            .mount(&server)
            .await;

        let client = insecure_client(&server).await;
        let gitlab = GitLab { inner: client };
        let endpoint = MergeRequests::builder().project(42).build().unwrap();
        let op = gitlab.paged_op(endpoint, Pagination::Limit(1));

        let result = op.execute(&ctx()).await.unwrap();
        let items = result.as_array().unwrap();
        assert_eq!(items.len(), 1);
    }

    #[tokio::test]
    async fn paged_op_kind_and_input() {
        let server = MockServer::start().await;
        let client = insecure_client(&server).await;
        let gitlab = GitLab { inner: client };
        let endpoint = MergeRequests::builder().project(42).build().unwrap();
        let op = gitlab.paged_op(endpoint, Pagination::Limit(5));

        assert_eq!(op.kind(), "gitlab");
        let input = op.input().unwrap();
        assert_eq!(input["endpoint"], "projects/42/merge_requests");
        assert_eq!(input["pagination"], "limit(5)");
    }
}
