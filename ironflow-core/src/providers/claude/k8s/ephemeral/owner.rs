//! The worker pod owning the agent pods and prompt ConfigMaps
//! ([`K8sEphemeralProvider::owner_pod`]), and the release of what it owns
//! ([`K8sEphemeralProvider::release_owned`]).
//!
//! Each object carries an `ownerReference` to the worker pod: the Kubernetes
//! garbage collector deletes it once the worker pod is gone. An owner
//! reference only holds in the namespace of its owner, so the owner is
//! looked up in the provider's namespace before any object is created: an
//! owner the garbage collector cannot find there would delete every object
//! right after its creation.

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference};
use kube::Error as KubeError;
use kube::api::Api;
use tracing::info;

use crate::error::AgentError;
use crate::provider::validate_owner_pod;

use super::super::cleanup::{delete_and_wait, owner_selection};
use super::super::common::create_client;
use super::K8sEphemeralProvider;

/// Name and uid of the worker pod owning what the provider creates.
#[derive(Debug, Clone)]
pub(in super::super) struct OwnerPod {
    name: String,
    uid: String,
}

fn failure(stderr: String) -> AgentError {
    AgentError::ProcessFailed {
        exit_code: -1,
        stderr,
    }
}

impl OwnerPod {
    /// Refuse an empty name or uid, before any request.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when [`validate_owner_pod`]
    /// refuses the owner.
    pub(in super::super) fn validate(&self) -> Result<(), AgentError> {
        validate_owner_pod(&self.name, &self.uid).map_err(failure)
    }

    /// The `ownerReference` set on every object the provider creates.
    ///
    /// `blockOwnerDeletion` stays `false`: `true` needs `update` on
    /// `pods/finalizers` under the `OwnerReferencesPermissionEnforcement`
    /// admission plugin.
    pub(in super::super) fn reference(&self) -> OwnerReference {
        OwnerReference {
            api_version: "v1".to_string(),
            kind: "Pod".to_string(),
            name: self.name.clone(),
            uid: self.uid.clone(),
            controller: Some(false),
            block_owner_deletion: Some(false),
        }
    }

    /// Check that the owner runs in the namespace of `pods` with its uid.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when the pod is not in the
    /// namespace, carries another uid, or cannot be read.
    pub(in super::super) async fn check_present(
        &self,
        pods: &Api<Pod>,
        namespace: &str,
    ) -> Result<(), AgentError> {
        let name = &self.name;
        let pod = match pods.get(name).await {
            Ok(pod) => pod,
            Err(KubeError::Api(e)) if e.code == 404 => {
                return Err(failure(format!(
                    "owner pod '{name}' not found in namespace '{namespace}': an ownerReference only \
                     holds in the namespace of its owner, so the worker must run in the namespace \
                     of the pods it creates"
                )));
            }
            Err(e) => return Err(failure(format!("failed to read owner pod '{name}': {e}"))),
        };
        match pod.metadata.uid.as_deref() {
            Some(uid) if uid == self.uid => Ok(()),
            actual => Err(failure(format!(
                "owner pod '{name}' has uid '{}', not '{}': pass the uid of the running worker pod \
                 (downward API metadata.uid)",
                actual.unwrap_or("<none>"),
                self.uid
            ))),
        }
    }
}

/// Whether one of the `ownerReferences` of `meta` carries a uid accepted by
/// `owned`.
pub(in super::super) fn has_owner(meta: &ObjectMeta, owned: impl Fn(&str) -> bool) -> bool {
    meta.owner_references
        .iter()
        .flatten()
        .any(|reference| owned(&reference.uid))
}

impl K8sEphemeralProvider {
    /// Tie every agent pod and prompt ConfigMap to the worker pod `name` of
    /// uid `uid`: each carries an `ownerReference` to it (`controller: false`,
    /// `blockOwnerDeletion: false`), and the Kubernetes garbage collector
    /// deletes it once the worker pod is gone. A worker killed during a step
    /// no longer leaves its agent running until the lease expires.
    ///
    /// The worker reads its name and uid through the downward API
    /// (`metadata.name`, `metadata.uid`). An owner reference only holds in
    /// the namespace of its owner: the worker pod must run in the
    /// [`namespace`](Self::namespace) of the provider. Before creating
    /// anything, each invocation reads the owner pod there and fails when it
    /// is missing or carries another uid, rather than creating a pod the
    /// garbage collector would delete at once.
    ///
    /// A restart of the worker container in the same pod (OOM, crash) keeps
    /// the pod, so its agents keep running: call
    /// [`release_owned`](Self::release_owned) at boot.
    ///
    /// An empty or whitespace-only `name` or `uid` fails every invocation,
    /// before any request. Without this call, no `ownerReference` is set.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::env::{VarError, var};
    ///
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// # fn example() -> Result<(), VarError> {
    /// // POD_NAME and POD_UID set from metadata.name and metadata.uid.
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .owner_pod(&var("POD_NAME")?, &var("POD_UID")?);
    /// # Ok(())
    /// # }
    /// ```
    pub fn owner_pod(mut self, name: &str, uid: &str) -> Self {
        self.owner = Some(OwnerPod {
            name: name.to_string(),
            uid: uid.to_string(),
        });
        self
    }

    /// Delete what the [`owner_pod`](Self::owner_pod) owns in the provider's
    /// namespace, and wait until its pods are gone.
    ///
    /// Call it at boot, before the worker takes any run. A worker container
    /// restarted in the same pod (OOM, `kill 1`) keeps its pod, so the
    /// garbage collector leaves its agents running; the new process has no
    /// step in flight, so every pod it owns is an orphan. Deleted: the ironflow
    /// pods (agents, `PodRun`) and the `JobRun` Jobs with an `ownerReference`
    /// to the owner's uid, the pods of those Jobs, and the prompt ConfigMaps
    /// with such a reference. Objects of another owner or without owner are
    /// left alone. Waits up to
    /// [`previous_attempt_timeout`](Self::previous_attempt_timeout).
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when no owner is set (nothing is
    /// deleted), the owner is refused or not running in the namespace with
    /// its uid, the objects cannot be listed or deleted, or a pod is still
    /// there after the timeout. A 403 on the Jobs is logged and skipped.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::error::AgentError;
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// # async fn example(name: &str, uid: &str) -> Result<(), AgentError> {
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .namespace("ironflow")
    ///     .owner_pod(name, uid);
    /// provider.release_owned().await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn release_owned(&self) -> Result<(), AgentError> {
        let owner = self.owner.as_ref().ok_or_else(|| {
            failure("release_owned needs owner_pod: no owner to release".to_string())
        })?;
        owner.validate()?;
        let client = create_client(&self.cluster_config).await?;
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);
        owner.check_present(&pods, &self.namespace).await?;
        let selection = owner_selection(&owner.name, &owner.uid);
        let limit = self.previous_attempt_timeout;
        let deleted = delete_and_wait(&client, &self.namespace, &selection, limit).await?;
        if deleted > 0 {
            info!(owner = %owner.name, pods_deleted = deleted, "released the pods of a previous worker process");
        }
        Ok(())
    }
}
