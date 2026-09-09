//! Verb marker types for [`KubeOp`](crate::KubeOp).
//!
//! Each verb type captures a specific Kubernetes API action. Verbs implement
//! the [`Verb`] trait, which provides a display name used in step metadata.
//!
//! # Supported verbs
//!
//! | Verb | Description |
//! |------|-------------|
//! | [`List`] | List resources matching optional label/field selectors |
//! | [`Get`] | Get a single resource by name |
//! | [`Create`] | Create a new resource |
//! | [`Update`] | Replace an existing resource |
//! | [`Patch`] | Patch an existing resource (JSON Merge, Strategic Merge, or Apply) |
//! | [`Delete`] | Delete a single resource by name |
//! | [`DeleteCollection`] | Delete all resources matching optional selectors |

use kube::api::{DeleteParams, ListParams, Patch as KubePatch, PatchParams, PostParams};
use serde::Serialize;

/// Trait implemented by all verb marker types.
///
/// Provides a human-readable name for step tracking and input metadata.
pub trait Verb: Send + Sync {
    /// Short lowercase name of this verb (e.g. `"list"`, `"get"`, `"create"`).
    fn name(&self) -> &'static str;
}

/// List resources matching optional label/field selectors.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::verb::List;
/// use kube::api::ListParams;
///
/// let list = List::default();
/// let filtered = List::new(ListParams::default().labels("app=nginx"));
/// ```
#[derive(Default)]
pub struct List {
    /// Parameters controlling the list operation.
    pub params: ListParams,
}

impl List {
    /// Create a list verb with the given [`ListParams`].
    pub fn new(params: ListParams) -> Self {
        Self { params }
    }
}

impl Verb for List {
    fn name(&self) -> &'static str {
        "list"
    }
}

/// Get a single resource by name.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::verb::Get;
///
/// let get = Get::new("my-pod");
/// ```
pub struct Get {
    /// Name of the resource to retrieve.
    pub name: String,
}

impl Get {
    /// Create a get verb targeting the named resource.
    pub fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Verb for Get {
    fn name(&self) -> &'static str {
        "get"
    }
}

/// Create a new resource.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::verb::Create;
/// use k8s_openapi::api::core::v1::Pod;
/// use kube::api::PostParams;
///
/// let pod = Pod::default();
/// let create = Create::with_data(pod);
/// ```
pub struct Create<T> {
    /// Parameters controlling the create operation.
    pub params: PostParams,
    /// The resource to create.
    pub data: T,
}

impl<T> Create<T> {
    /// Create a create verb with the given parameters and resource data.
    pub fn new(params: PostParams, data: T) -> Self {
        Self { params, data }
    }

    /// Create a create verb with default parameters and the given resource data.
    pub fn with_data(data: T) -> Self {
        Self {
            params: PostParams::default(),
            data,
        }
    }
}

impl<T: Send + Sync> Verb for Create<T> {
    fn name(&self) -> &'static str {
        "create"
    }
}

/// Replace an existing resource.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::verb::Update;
/// use k8s_openapi::api::core::v1::Pod;
/// use kube::api::PostParams;
///
/// let pod = Pod::default();
/// let update = Update::with_data(pod);
/// ```
pub struct Update<T> {
    /// Parameters controlling the update operation.
    pub params: PostParams,
    /// The full resource to replace.
    pub data: T,
}

impl<T> Update<T> {
    /// Create an update verb with the given parameters and resource data.
    pub fn new(params: PostParams, data: T) -> Self {
        Self { params, data }
    }

    /// Create an update verb with default parameters and the given resource data.
    pub fn with_data(data: T) -> Self {
        Self {
            params: PostParams::default(),
            data,
        }
    }
}

impl<T: Send + Sync> Verb for Update<T> {
    fn name(&self) -> &'static str {
        "update"
    }
}

/// Patch an existing resource.
///
/// Supports JSON Merge Patch, Strategic Merge Patch, and Server-Side Apply
/// via [`kube::api::Patch`].
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::verb::Patch;
/// use kube::api::{PatchParams, Patch as KubePatch};
/// use serde_json::json;
///
/// let patch = Patch::new(
///     "my-pod",
///     PatchParams::default(),
///     KubePatch::Merge(json!({"metadata": {"labels": {"env": "prod"}}})),
/// );
/// ```
pub struct Patch<P: Serialize> {
    /// Name of the resource to patch.
    pub name: String,
    /// Parameters controlling the patch operation.
    pub params: PatchParams,
    /// The patch body.
    pub patch: KubePatch<P>,
}

impl<P: Serialize> Patch<P> {
    /// Create a patch verb targeting the named resource.
    pub fn new(name: impl Into<String>, params: PatchParams, patch: KubePatch<P>) -> Self {
        Self {
            name: name.into(),
            params,
            patch,
        }
    }
}

impl<P: Serialize + Send + Sync> Verb for Patch<P> {
    fn name(&self) -> &'static str {
        "patch"
    }
}

/// Delete a single resource by name.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::verb::Delete;
///
/// let delete = Delete::new("my-pod");
/// ```
pub struct Delete {
    /// Name of the resource to delete.
    pub name: String,
    /// Parameters controlling the delete operation.
    pub params: DeleteParams,
}

impl Delete {
    /// Create a delete verb targeting the named resource with default parameters.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            params: DeleteParams::default(),
        }
    }

    /// Create a delete verb with explicit parameters.
    pub fn with_params(name: impl Into<String>, params: DeleteParams) -> Self {
        Self {
            name: name.into(),
            params,
        }
    }
}

impl Verb for Delete {
    fn name(&self) -> &'static str {
        "delete"
    }
}

/// Delete all resources matching optional selectors.
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_k8s::verb::DeleteCollection;
/// use kube::api::ListParams;
///
/// let dc = DeleteCollection::default();
/// let filtered = DeleteCollection::new(ListParams::default().labels("app=nginx"));
/// ```
#[derive(Default)]
pub struct DeleteCollection {
    /// Selectors to filter which resources to delete.
    pub list_params: ListParams,
    /// Parameters controlling the delete operation.
    pub delete_params: DeleteParams,
}

impl DeleteCollection {
    /// Create a delete-collection verb with the given list params and default delete params.
    pub fn new(list_params: ListParams) -> Self {
        Self {
            list_params,
            delete_params: DeleteParams::default(),
        }
    }

    /// Create a delete-collection verb with explicit list and delete params.
    pub fn with_params(list_params: ListParams, delete_params: DeleteParams) -> Self {
        Self {
            list_params,
            delete_params,
        }
    }
}

impl Verb for DeleteCollection {
    fn name(&self) -> &'static str {
        "delete_collection"
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::Pod;
    use kube::api::{DeleteParams, ListParams, Patch as KubePatch, PatchParams, PostParams};
    use serde_json::json;

    use super::*;

    #[test]
    fn list_default() {
        let list = List::default();
        assert_eq!(list.name(), "list");
    }

    #[test]
    fn list_with_params() {
        let list = List::new(ListParams::default().labels("app=nginx"));
        assert_eq!(list.name(), "list");
    }

    #[test]
    fn get_name() {
        let get = Get::new("my-pod");
        assert_eq!(get.name(), "get");
        assert_eq!(get.name, "my-pod");
    }

    #[test]
    fn create_with_data() {
        let create = Create::with_data(Pod::default());
        assert_eq!(create.name(), "create");
    }

    #[test]
    fn create_with_params() {
        let create = Create::new(PostParams::default(), Pod::default());
        assert_eq!(create.name(), "create");
    }

    #[test]
    fn update_with_data() {
        let update = Update::with_data(Pod::default());
        assert_eq!(update.name(), "update");
    }

    #[test]
    fn update_with_params() {
        let update = Update::new(PostParams::default(), Pod::default());
        assert_eq!(update.name(), "update");
    }

    #[test]
    fn patch_name() {
        let patch = Patch::new("p", PatchParams::default(), KubePatch::Merge(json!({})));
        assert_eq!(patch.name(), "patch");
        assert_eq!(patch.name, "p");
    }

    #[test]
    fn delete_name() {
        let delete = Delete::new("my-pod");
        assert_eq!(delete.name(), "delete");
        assert_eq!(delete.name, "my-pod");
    }

    #[test]
    fn delete_with_params() {
        let delete = Delete::with_params("p", DeleteParams::default());
        assert_eq!(delete.name(), "delete");
    }

    #[test]
    fn delete_collection_default() {
        let dc = DeleteCollection::default();
        assert_eq!(dc.name(), "delete_collection");
    }

    #[test]
    fn delete_collection_with_params() {
        let dc = DeleteCollection::with_params(
            ListParams::default().labels("app=old"),
            DeleteParams::default(),
        );
        assert_eq!(dc.name(), "delete_collection");
    }
}
