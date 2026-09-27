//! [`GitLabPagedOp`] -- executes a paginated [`Endpoint`] across every requested page as a
//! tracked [`Operation`].

use async_trait::async_trait;
use gitlab::AsyncGitlab;
use gitlab::api::{AsyncQuery, Endpoint, Pageable, Paged, Pagination, paged};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

/// A paginated GitLab endpoint wrapped as an Ironflow [`Operation`].
///
/// Created via [`GitLab::paged_op`](crate::GitLab::paged_op). Unlike [`GitLabOp`](crate::GitLabOp),
/// which issues a single request, this drives pagination itself and concatenates every page
/// requested by `pagination` into a single JSON array.
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
///
/// # Errors
///
/// [`Operation::execute`] returns [`OperationError::Http`] if any page request fails.
pub struct GitLabPagedOp<E> {
    client: AsyncGitlab,
    paged: Paged<E>,
    endpoint_path: String,
    pagination: Pagination,
}

impl<E> GitLabPagedOp<E>
where
    E: Endpoint,
{
    pub(crate) fn new(client: AsyncGitlab, endpoint: E, pagination: Pagination) -> Self {
        let endpoint_path = endpoint.endpoint().into_owned();
        let paged = paged(endpoint, pagination);
        Self {
            client,
            paged,
            endpoint_path,
            pagination,
        }
    }
}

#[async_trait]
impl<E> Operation for GitLabPagedOp<E>
where
    E: Endpoint + Pageable + Sync + Send,
{
    fn kind(&self) -> &str {
        "gitlab"
    }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let results: Vec<Value> =
            self.paged
                .query_async(&self.client)
                .await
                .map_err(|e| OperationError::Http {
                    status: None,
                    message: e.to_string(),
                })?;
        Ok(Value::Array(results))
    }

    fn input(&self) -> Option<Value> {
        let pagination = match self.pagination {
            Pagination::All => "all".to_string(),
            Pagination::AllPerPageLimit(n) => format!("all_per_page_limit({n})"),
            Pagination::Limit(n) => format!("limit({n})"),
            _ => "unknown".to_string(),
        };
        Some(Value::Object(serde_json::Map::from_iter([
            (
                "endpoint".to_string(),
                Value::String(self.endpoint_path.clone()),
            ),
            ("pagination".to_string(), Value::String(pagination)),
        ])))
    }
}
