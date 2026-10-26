//! [`GitLab`] client built from an [`OperationContext`]'s secret store.

use gitlab::api::{Endpoint, Pageable, Pagination};
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use ironflow_ops_common::redact::Redactor;

use crate::operation::GitLabOp;
use crate::paged_operation::GitLabPagedOp;
use crate::trace_operation::GitLabTraceOp;

/// A GitLab client that resolves credentials from the workflow's secret store.
///
/// Wraps [`AsyncGitlab`] and provides a convenience [`op`](GitLab::op) method
/// to turn any endpoint into a tracked [`Operation`](ironflow_core::operation::Operation).
///
/// Every operation it creates masks secrets in the request body before
/// sending it: the token the client was built with, the values added with
/// [`with_secrets`](GitLab::with_secrets), and token-shaped strings
/// (`sk-ant-...`, `glpat-...`). See [`GitLabOp`].
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
    redactor: Redactor,
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
    /// The operations it creates mask `token` in their request body.
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
        Ok(Self::with_token(inner, token))
    }

    /// A client whose operations mask `token`, the one `inner` was built with.
    fn with_token(inner: AsyncGitlab, token: &str) -> Self {
        Self::from(inner).with_secrets([token])
    }

    /// Add secret values to mask in the body of every operation this client
    /// creates from now on.
    ///
    /// Values shorter than 8 characters are ignored. The token passed to
    /// [`GitLab::new`] or read by [`GitLab::from_context`] is already masked.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    ///
    /// # async fn example(worker_token: String) -> Result<(), ironflow_core::error::OperationError> {
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com")
    ///     .await?
    ///     .with_secrets([worker_token]);
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_secrets<I, S>(mut self, secrets: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.redactor = self.redactor.with_secrets(secrets);
        self
    }

    /// The underlying [`AsyncGitlab`] client.
    ///
    /// Use this with [`AsyncQuery::query_async`](gitlab::api::AsyncQuery::query_async)
    /// for typed endpoint calls. A query sent this way bypasses the secret
    /// masking of [`GitLab::op`]: publish text through `op` instead.
    pub fn client(&self) -> &AsyncGitlab {
        &self.inner
    }

    /// Wrap an endpoint as a tracked [`Operation`](ironflow_core::operation::Operation).
    ///
    /// The returned [`GitLabOp`] implements `Operation` so it can be passed
    /// to `WorkflowContext::operation()` for step lifecycle tracking. It masks
    /// the client's known secrets in the request body.
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
        GitLabOp::new(self.inner.clone(), endpoint, self.redactor.clone())
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

    /// Wrap a job trace endpoint as a tracked [`Operation`](ironflow_core::operation::Operation)
    /// that returns the trace as text.
    ///
    /// The trace is plain text, not JSON, so [`GitLab::op`] cannot read it. The
    /// returned [`GitLabTraceOp`] yields a JSON string (read it with
    /// `StepOutput::text()`), raw by default, or decoded / condensed with
    /// [`GitLabTraceOp::decoded`] and [`GitLabTraceOp::condensed`]. No secret
    /// is masked: a GET has no body, and GitLab already masks CI variables in
    /// traces.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_gitlab::GitLab;
    /// use ironflow_ops_gitlab::endpoints::jobs::JobTrace;
    /// use gitlab::api::common::NameOrId;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
    /// let endpoint = JobTrace { project: NameOrId::from(42), job: 7 };
    /// let op = gitlab.trace_op(endpoint).condensed(8000, "[... trace truncated ...]");
    /// # Ok(())
    /// # }
    /// ```
    pub fn trace_op<E>(&self, endpoint: E) -> GitLabTraceOp<E>
    where
        E: Endpoint + Send + Sync,
    {
        GitLabTraceOp::new(self.inner.clone(), endpoint)
    }
}

/// Build a [`GitLab`] client from an already configured [`AsyncGitlab`].
///
/// Use this when [`GitLab::new`] is not flexible enough -- for example to
/// point the client at an HTTP test double (wiremock) or a self-hosted
/// instance with custom TLS settings, by building the client directly via
/// [`GitlabBuilder`] and converting the result.
///
/// The token of an [`AsyncGitlab`] cannot be read back: add it with
/// [`GitLab::with_secrets`] to mask it. Token-shaped strings are masked
/// either way.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use gitlab::GitlabBuilder;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // `.insecure()` allows plain HTTP, e.g. against a wiremock server in tests.
/// let client = GitlabBuilder::new("gitlab.example.com", "glpat-xxxx")
///     .insecure()
///     .build_async()
///     .await?;
/// let gitlab = GitLab::from(client).with_secrets(["glpat-xxxx"]);
/// # Ok(())
/// # }
/// ```
impl From<AsyncGitlab> for GitLab {
    fn from(inner: AsyncGitlab) -> Self {
        Self {
            inner,
            redactor: Redactor::default(),
        }
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

    use gitlab::GitlabBuilder;
    use gitlab::api::projects;
    use gitlab::api::projects::merge_requests::MergeRequests;
    use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
    use serde_json::{Value, json};
    use wiremock::matchers::{body_string, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::endpoints::merge_requests::CreateMergeRequestDiscussionNote;

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
    async fn client_token_reaches_gitlab_masked() {
        let server = MockServer::start().await;
        // Not token-shaped: only the client's own token list can mask it.
        let token = "plain-client-token-1234";
        Mock::given(method("POST"))
            .and(path(
                "/api/v4/projects/42/merge_requests/7/discussions/abcd1234/notes",
            ))
            .and(body_string("body=leaked+%5BREDACTED%5D"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"id": 1})))
            .expect(1)
            .mount(&server)
            .await;

        let gitlab = GitLab::with_token(insecure_client(&server).await, token);
        let op = gitlab.op(CreateMergeRequestDiscussionNote {
            project: 42.into(),
            merge_request: 7,
            discussion_id: "abcd1234".to_string(),
            body: format!("leaked {token}"),
        });

        assert!(op.masks_secrets());
        let result = op.execute(&ctx()).await.unwrap();
        assert_eq!(result["id"], 1);
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
    async fn from_async_gitlab_hits_mock_server() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v4/user"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(r#"{"id":1,"username":"demo"}"#),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v4/projects/42"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":42,"name":"demo"}"#))
            .mount(&server)
            .await;

        let client = GitlabBuilder::new(server.address().to_string(), "test-token")
            .insecure()
            .build_async()
            .await
            .unwrap();
        let gitlab: GitLab = client.into();

        let endpoint = projects::Project::builder().project(42).build().unwrap();
        let op = gitlab.op(endpoint);
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let result = op.execute(&ctx).await.unwrap();

        assert_eq!(result["id"], 42);
        assert_eq!(result["name"], "demo");

        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        let paths: Vec<&str> = requests.iter().map(|r| r.url.path()).collect();
        assert!(paths.contains(&"/api/v4/user"));
        assert!(paths.contains(&"/api/v4/projects/42"));
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
        let gitlab = GitLab::from(client);
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
        let gitlab = GitLab::from(client);
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
        let gitlab = GitLab::from(client);
        let endpoint = MergeRequests::builder().project(42).build().unwrap();
        let op = gitlab.paged_op(endpoint, Pagination::Limit(5));

        assert_eq!(op.kind(), "gitlab");
        let input = op.input().unwrap();
        assert_eq!(input["endpoint"], "projects/42/merge_requests");
        assert_eq!(input["pagination"], "limit(5)");
    }
}
