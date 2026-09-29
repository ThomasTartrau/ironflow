//! Repository routes missing from the `gitlab` crate:
//! `GET /projects/:id/repository/merge_base`.

use std::borrow::Cow;

use gitlab::api::common::NameOrId;
use gitlab::api::endpoint_prelude::{Endpoint, Method, QueryParams};

/// Find the common ancestor of two or more refs (branch names, tags or SHAs).
///
/// Maps to `GET /projects/:id/repository/merge_base?refs[]=a&refs[]=b`.
/// The answer is the merge base commit (`id`, `short_id`, ...). Comparing it
/// with one of the refs tells whether that ref is an ancestor of the others.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::repository::MergeBase;
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = MergeBase {
///     project: NameOrId::from("group/project"),
///     refs: vec!["0a1b2c3".to_string(), "main".to_string()],
/// };
/// let op = gitlab.op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails, for instance when the refs share no
/// history (GitLab answers 400) or a ref does not exist.
pub struct MergeBase {
    /// The project holding the refs.
    pub project: NameOrId<'static>,
    /// The refs to find the common ancestor of (at least two).
    pub refs: Vec<String>,
}

impl Endpoint for MergeBase {
    fn method(&self) -> Method {
        Method::GET
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!("projects/{}/repository/merge_base", self.project).into()
    }

    fn parameters(&self) -> QueryParams<'_> {
        let mut params = QueryParams::default();
        params.extend(self.refs.iter().map(|r| ("refs[]", r.as_str())));
        params
    }
}
