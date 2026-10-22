//! [`PodRun::owner_pod`] and [`JobRun::owner_pod`](crate::job_run::JobRun::owner_pod):
//! the pod or Job carries an `ownerReference` to the worker pod, so the
//! Kubernetes garbage collector deletes it once the worker pod is gone.
//!
//! An owner reference only holds in the namespace of its owner: `run()`
//! reads the owner pod in the namespace of the run before creating
//! anything, rather than creating an object the garbage collector would
//! delete at once.

use ironflow_core::error::OperationError;
use ironflow_core::provider::validate_owner_pod;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::OwnerReference;
use kube::api::Api;
use kube::{Client, Error as KubeError};

use super::PodRun;
use crate::error::k8s_external;

#[cfg(test)]
mod tests;

/// Name and uid of the worker pod owning a [`PodRun`] pod or a
/// [`JobRun`](crate::job_run::JobRun) Job.
#[derive(Debug, Clone)]
pub(crate) struct OwnerPod {
    name: String,
    uid: String,
}

impl OwnerPod {
    pub(crate) fn new(name: &str, uid: &str) -> Self {
        Self {
            name: name.to_string(),
            uid: uid.to_string(),
        }
    }

    /// The `ownerReference` of the pod or Job. `blockOwnerDeletion` stays
    /// `false`: `true` needs `update` on `pods/finalizers`.
    pub(crate) fn reference(&self) -> OwnerReference {
        OwnerReference {
            api_version: "v1".to_string(),
            kind: "Pod".to_string(),
            name: self.name.clone(),
            uid: self.uid.clone(),
            controller: Some(false),
            block_owner_deletion: Some(false),
        }
    }

    /// Refuse an empty name or uid before any request, then check that the
    /// owner runs in `namespace` with its uid.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] with `origin: "kubernetes"` when
    /// the name or uid is empty, the pod is not in `namespace`, carries
    /// another uid, or cannot be read.
    pub(crate) async fn check(
        &self,
        client: &Client,
        namespace: &str,
    ) -> Result<(), OperationError> {
        validate_owner_pod(&self.name, &self.uid).map_err(k8s_external)?;
        let name = &self.name;
        let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
        let pod = match pods.get(name).await {
            Ok(pod) => pod,
            Err(KubeError::Api(e)) if e.code == 404 => {
                return Err(k8s_external(format!(
                    "owner pod '{name}' not found in namespace '{namespace}': an ownerReference only \
                     holds in the namespace of its owner, so the worker must run in the namespace \
                     of the objects it creates"
                )));
            }
            Err(e) => {
                return Err(k8s_external(format!(
                    "failed to read owner pod '{name}': {e}"
                )));
            }
        };
        match pod.metadata.uid.as_deref() {
            Some(uid) if uid == self.uid => Ok(()),
            actual => Err(k8s_external(format!(
                "owner pod '{name}' has uid '{}', not '{}': pass the uid of the running worker pod \
                 (downward API metadata.uid)",
                actual.unwrap_or("<none>"),
                self.uid
            ))),
        }
    }
}

impl PodRun {
    /// Tie the pod to the worker pod `name` of uid `uid`: the pod carries an
    /// `ownerReference` to it (`controller: false`, `blockOwnerDeletion:
    /// false`), and the Kubernetes garbage collector deletes it once the
    /// worker pod is gone, instead of letting it run until its deadline.
    ///
    /// The worker reads its name and uid through the downward API
    /// (`metadata.name`, `metadata.uid`), and must run in the
    /// [`namespace`](Self::namespace) of the pod: [`run`](Self::run) reads
    /// the owner pod there first and fails, before creating anything, when
    /// it is missing, carries another uid, or `name` or `uid` is empty.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_k8s::pod_run::PodRun;
    /// # use ironflow_ops_k8s::KubeClient;
    ///
    /// # fn example(kube: &KubeClient, name: &str, uid: &str) {
    /// // name and uid read from POD_NAME and POD_UID (downward API).
    /// let run = PodRun::new(kube, "check", "busybox", "make check")
    ///     .owner_pod(name, uid);
    /// # let _ = run;
    /// # }
    /// ```
    #[must_use]
    pub fn owner_pod(mut self, name: &str, uid: &str) -> Self {
        self.owner = Some(OwnerPod::new(name, uid));
        self
    }
}
