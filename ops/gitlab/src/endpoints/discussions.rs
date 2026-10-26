//! Issue and commit discussion routes missing from the `gitlab` crate:
//! `GET|POST /projects/:id/issues/:iid/discussions`,
//! `GET /projects/:id/issues/:iid/discussions/:discussion_id`,
//! `POST /projects/:id/issues/:iid/discussions/:discussion_id/notes` and the same four
//! routes under `/projects/:id/repository/commits/:sha`.
//!
//! Resolving is not offered: GitLab only resolves merge request and diff discussions.

use std::borrow::Cow;

use gitlab::api::Pageable;
use gitlab::api::common::NameOrId;
use gitlab::api::endpoint_prelude::{BodyError, Endpoint, FormParams, Method};

/// The object a discussion thread is attached to.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::endpoints::discussions::DiscussionObject;
///
/// let issue = DiscussionObject::Issue(7);
/// let commit = DiscussionObject::Commit("0a1b2c3".to_string());
/// ```
pub enum DiscussionObject {
    /// An issue, by `iid`.
    Issue(u64),
    /// A commit, by SHA or ref.
    Commit(String),
}

impl DiscussionObject {
    /// Base path of the object, without a trailing slash. The commit SHA is percent-encoded.
    fn path(&self, project: &NameOrId<'static>) -> String {
        match self {
            Self::Issue(iid) => format!("projects/{project}/issues/{iid}"),
            Self::Commit(sha) => format!(
                "projects/{project}/repository/commits/{}",
                NameOrId::from(sha.clone()),
            ),
        }
    }
}

/// List the discussion threads of an issue or a commit.
///
/// Maps to `GET /projects/:id/issues/:iid/discussions` or
/// `GET /projects/:id/repository/commits/:sha/discussions`. Use
/// [`GitLab::paged_op`](crate::GitLab::paged_op) to fetch every page.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::discussions::{DiscussionObject, Discussions};
/// use gitlab::api::Pagination;
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = Discussions {
///     project: NameOrId::from(42),
///     object: DiscussionObject::Issue(7),
/// };
/// let op = gitlab.paged_op(endpoint, Pagination::All);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails or the response cannot be deserialized (e.g. the
/// project, issue or commit does not exist).
pub struct Discussions {
    /// The project the object belongs to.
    pub project: NameOrId<'static>,
    /// The issue or commit holding the discussions.
    pub object: DiscussionObject,
}

impl Endpoint for Discussions {
    fn method(&self) -> Method {
        Method::GET
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!("{}/discussions", self.object.path(&self.project)).into()
    }
}

impl Pageable for Discussions {}

/// Read one discussion thread of an issue or a commit.
///
/// Maps to `GET /projects/:id/issues/:iid/discussions/:discussion_id` or
/// `GET /projects/:id/repository/commits/:sha/discussions/:discussion_id`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::discussions::{Discussion, DiscussionObject};
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = Discussion {
///     project: NameOrId::from(42),
///     object: DiscussionObject::Commit("0a1b2c3".to_string()),
///     discussion_id: "abcd1234".to_string(),
/// };
/// let op = gitlab.op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails or the response cannot be deserialized (e.g. the
/// discussion does not exist).
pub struct Discussion {
    /// The project the object belongs to.
    pub project: NameOrId<'static>,
    /// The issue or commit holding the discussion.
    pub object: DiscussionObject,
    /// The ID of the discussion thread.
    pub discussion_id: String,
}

impl Endpoint for Discussion {
    fn method(&self) -> Method {
        Method::GET
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!(
            "{}/discussions/{}",
            self.object.path(&self.project),
            NameOrId::from(self.discussion_id.clone()),
        )
        .into()
    }
}

/// Start a new discussion thread on an issue or a commit.
///
/// Maps to `POST /projects/:id/issues/:iid/discussions` or
/// `POST /projects/:id/repository/commits/:sha/discussions`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::discussions::{CreateDiscussion, DiscussionObject};
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = CreateDiscussion {
///     project: NameOrId::from(42),
///     object: DiscussionObject::Issue(7),
///     body: "Can you reproduce this?".to_string(),
/// };
/// let op = gitlab.op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails or the response cannot be deserialized (e.g. the
/// project, issue or commit does not exist).
pub struct CreateDiscussion {
    /// The project the object belongs to.
    pub project: NameOrId<'static>,
    /// The issue or commit to discuss.
    pub object: DiscussionObject,
    /// The text of the first note.
    pub body: String,
}

impl Endpoint for CreateDiscussion {
    fn method(&self) -> Method {
        Method::POST
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!("{}/discussions", self.object.path(&self.project)).into()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, BodyError> {
        let mut params = FormParams::default();
        params.push("body", self.body.as_str());
        params.into_body()
    }
}

/// Reply to an existing discussion thread of an issue or a commit.
///
/// Maps to `POST /projects/:id/issues/:iid/discussions/:discussion_id/notes` or
/// `POST /projects/:id/repository/commits/:sha/discussions/:discussion_id/notes`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::discussions::{DiscussionObject, ReplyToDiscussion};
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = ReplyToDiscussion {
///     project: NameOrId::from(42),
///     object: DiscussionObject::Issue(7),
///     discussion_id: "abcd1234".to_string(),
///     body: "Fixed in the latest commit.".to_string(),
/// };
/// let op = gitlab.op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails or the response cannot be deserialized (e.g. the
/// discussion does not exist).
pub struct ReplyToDiscussion {
    /// The project the object belongs to.
    pub project: NameOrId<'static>,
    /// The issue or commit holding the discussion.
    pub object: DiscussionObject,
    /// The ID of the discussion thread to reply to.
    pub discussion_id: String,
    /// The text of the note.
    pub body: String,
}

impl Endpoint for ReplyToDiscussion {
    fn method(&self) -> Method {
        Method::POST
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!(
            "{}/discussions/{}/notes",
            self.object.path(&self.project),
            NameOrId::from(self.discussion_id.clone()),
        )
        .into()
    }

    fn body(&self) -> Result<Option<(&'static str, Vec<u8>)>, BodyError> {
        let mut params = FormParams::default();
        params.push("body", self.body.as_str());
        params.into_body()
    }
}
