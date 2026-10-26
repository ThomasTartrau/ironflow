//! Release route missing from the `gitlab` crate:
//! `GET /projects/:id/releases`.

use std::borrow::Cow;

use gitlab::api::common::NameOrId;
use gitlab::api::endpoint_prelude::{Endpoint, Method, QueryParams};

/// Field to order releases by.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::endpoints::releases::ReleaseOrderBy;
///
/// let order_by = ReleaseOrderBy::ReleasedAt;
/// ```
pub enum ReleaseOrderBy {
    /// Order by release date (`released_at`).
    ReleasedAt,
    /// Order by creation date (`created_at`).
    CreatedAt,
}

impl ReleaseOrderBy {
    /// The value GitLab expects in `order_by`.
    fn as_str(&self) -> &'static str {
        match self {
            Self::ReleasedAt => "released_at",
            Self::CreatedAt => "created_at",
        }
    }
}

/// Sort direction of a release listing.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::endpoints::releases::ReleaseSort;
///
/// let sort = ReleaseSort::Desc;
/// ```
pub enum ReleaseSort {
    /// Oldest first.
    Asc,
    /// Newest first.
    Desc,
}

impl ReleaseSort {
    /// The value GitLab expects in `sort`.
    fn as_str(&self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// List the releases of a project.
///
/// Maps to `GET /projects/:id/releases`. This endpoint is not [`Pageable`](gitlab::api::Pageable):
/// it carries its own `per_page`, so [`GitLab::op`](crate::GitLab::op) returns one page of
/// `per_page` items (GitLab's default when `None`).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_gitlab::GitLab;
/// use ironflow_ops_gitlab::endpoints::releases::{ReleaseOrderBy, ReleaseSort, Releases};
/// use gitlab::api::common::NameOrId;
///
/// # async fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let gitlab = GitLab::new("glpat-xxxx", "gitlab.com").await?;
/// let endpoint = Releases {
///     project: NameOrId::from("group/project"),
///     order_by: Some(ReleaseOrderBy::ReleasedAt),
///     sort: Some(ReleaseSort::Desc),
///     per_page: Some(5),
/// };
/// let op = gitlab.op(endpoint);
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns an error if the request fails or the response cannot be deserialized (e.g. the
/// project does not exist).
pub struct Releases {
    /// The project to list releases of.
    pub project: NameOrId<'static>,
    /// Field to order by, GitLab's default when `None`.
    pub order_by: Option<ReleaseOrderBy>,
    /// Sort direction, GitLab's default when `None`.
    pub sort: Option<ReleaseSort>,
    /// Page size, GitLab's default when `None`.
    pub per_page: Option<u64>,
}

impl Endpoint for Releases {
    fn method(&self) -> Method {
        Method::GET
    }

    fn endpoint(&self) -> Cow<'static, str> {
        format!("projects/{}/releases", self.project).into()
    }

    fn parameters(&self) -> QueryParams<'_> {
        let mut params = QueryParams::default();
        params.push_opt(
            "order_by",
            self.order_by.as_ref().map(ReleaseOrderBy::as_str),
        );
        params.push_opt("sort", self.sort.as_ref().map(ReleaseSort::as_str));
        params.push_opt("per_page", self.per_page);
        params
    }
}
