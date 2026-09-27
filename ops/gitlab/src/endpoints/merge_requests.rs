//! Merge request discussion routes missing from the `gitlab` crate:
//! `POST /projects/:id/merge_requests/:iid/discussions/:discussion_id/notes` and
//! `PUT /projects/:id/merge_requests/:iid/discussions/:discussion_id`.

use std::borrow::Cow;

use gitlab::api::common::NameOrId;
use gitlab::api::endpoint_prelude::{BodyError, Endpoint, FormParams, Method};

/// Add a note (reply) to an existing merge request discussion.
///
/// Maps to `POST /projects/:id/merge_requests/:merge_request/discussions/:discussion_id/notes`.
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
/// let endpoint = CreateMergeRequestDiscussionNote {
///     project: NameOrId::from(42),
///     merge_request: 7,
///     discussion_id: "abcd1234".to_string(),
///     body: "Looks good, thanks!".to_string(),
/// };
/// let op = gitlab.op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails or the response cannot be deserialized (e.g. the
/// project, merge request or discussion does not exist).
pub struct CreateMergeRequestDiscussionNote {
    /// The project the merge request belongs to.
    pub project: NameOrId<'static>,
    /// The internal ID (`iid`) of the merge request.
    pub merge_request: u64,
    /// The ID of the discussion thread to reply to.
    pub discussion_id: String,
    /// The text of the note.
    pub body: String,
}

impl Endpoint for CreateMergeRequestDiscussionNote {
    fn method(&self) -> Method {
        Method::POST
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!(
            "projects/{}/merge_requests/{}/discussions/{}/notes",
            self.project, self.merge_request, self.discussion_id,
        )
        .into()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, BodyError> {
        let mut params = FormParams::default();
        params.push("body", self.body.as_str());
        params.into_body()
    }
}

/// Resolve or unresolve an existing merge request discussion.
///
/// Maps to `PUT /projects/:id/merge_requests/:merge_request/discussions/:discussion_id`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::merge_requests::ResolveMergeRequestDiscussion;
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = ResolveMergeRequestDiscussion {
///     project: NameOrId::from(42),
///     merge_request: 7,
///     discussion_id: "abcd1234".to_string(),
///     resolved: true,
/// };
/// let op = gitlab.op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails or the response cannot be deserialized (e.g. the
/// project, merge request or discussion does not exist).
pub struct ResolveMergeRequestDiscussion {
    /// The project the merge request belongs to.
    pub project: NameOrId<'static>,
    /// The internal ID (`iid`) of the merge request.
    pub merge_request: u64,
    /// The ID of the discussion thread to resolve or unresolve.
    pub discussion_id: String,
    /// Whether the discussion should be marked resolved.
    pub resolved: bool,
}

impl Endpoint for ResolveMergeRequestDiscussion {
    fn method(&self) -> Method {
        Method::PUT
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!(
            "projects/{}/merge_requests/{}/discussions/{}",
            self.project, self.merge_request, self.discussion_id,
        )
        .into()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, BodyError> {
        let mut params = FormParams::default();
        params.push("resolved", self.resolved);
        params.into_body()
    }
}
