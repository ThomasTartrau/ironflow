//! Note route missing from the `gitlab` crate:
//! `GET /projects/:id/issues/:iid/notes/:note_id` and
//! `GET /projects/:id/merge_requests/:iid/notes/:note_id`.

use std::borrow::Cow;

use gitlab::api::common::NameOrId;
use gitlab::api::endpoint_prelude::{Endpoint, Method};

/// The kind of object a note is attached to.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::endpoints::notes::Noteable;
///
/// let issue = Noteable::Issue(7);
/// let merge_request = Noteable::MergeRequest(3);
/// ```
pub enum Noteable {
    /// An issue, by `iid`.
    Issue(u64),
    /// A merge request, by `iid`.
    MergeRequest(u64),
}

/// Read one note by id.
///
/// Maps to `GET /projects/:id/issues/:iid/notes/:note_id` or
/// `GET /projects/:id/merge_requests/:iid/notes/:note_id`. GitLab has no route addressing a
/// note by id alone: a note webhook payload gives the noteable type and its `iid`.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::notes::{Note, Noteable};
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = Note {
///     project: NameOrId::from(42),
///     noteable: Noteable::MergeRequest(7),
///     note: 1001,
/// };
/// let op = gitlab.op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails or the response cannot be deserialized (e.g. the
/// note does not exist, GitLab answers 404).
pub struct Note {
    /// The project the noteable belongs to.
    pub project: NameOrId<'static>,
    /// The issue or merge request holding the note.
    pub noteable: Noteable,
    /// The ID of the note.
    pub note: u64,
}

impl Endpoint for Note {
    fn method(&self) -> Method {
        Method::GET
    }

    fn endpoint(&self) -> Cow<'static, str> {
        let path = match self.noteable {
            Noteable::Issue(iid) => {
                format!(
                    "projects/{}/issues/{}/notes/{}",
                    self.project, iid, self.note
                )
            }
            Noteable::MergeRequest(iid) => format!(
                "projects/{}/merge_requests/{}/notes/{}",
                self.project, iid, self.note
            ),
        };
        path.into()
    }
}
