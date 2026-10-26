//! Job routes missing from the `gitlab` crate:
//! `GET /projects/:id/jobs/:job_id/trace`.

use std::borrow::Cow;

use gitlab::api::common::NameOrId;
use gitlab::api::endpoint_prelude::{Endpoint, Method};

/// Read the trace (the log) of a CI job.
///
/// Maps to `GET /projects/:id/jobs/:job_id/trace`. The answer is plain text,
/// not JSON: read it through [`GitLab::trace_op`](crate::GitLab::trace_op).
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
/// let endpoint = JobTrace {
///     project: NameOrId::from("group/project"),
///     job: 7,
/// };
/// let op = gitlab.trace_op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails, for instance when the job does not
/// exist (404) or the token cannot read it (403).
pub struct JobTrace {
    /// The project holding the job.
    pub project: NameOrId<'static>,
    /// The id of the job.
    pub job: u64,
}

impl Endpoint for JobTrace {
    fn method(&self) -> Method {
        Method::GET
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!("projects/{}/jobs/{}/trace", self.project, self.job).into()
    }
}
