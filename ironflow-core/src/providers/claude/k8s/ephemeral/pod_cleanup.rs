//! The pod and prompt ConfigMap of one step, deleted whatever ends the step.

use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use kube::api::{Api, DeleteParams};
use tokio::runtime::Handle;
use tracing::{debug, warn};

/// The pod of one step and its prompt ConfigMap, deleted once the step ends.
///
/// * a step that ends, in success, failure or after the provider timeout,
///   calls [`delete`](Self::delete);
/// * a step dropped first starts the deletion from [`Drop`], in a task of its
///   own: the step timed out (`AgentConfig::timeout`), the run was cancelled,
///   the lease was lost, the run timed out, or a parallel branch was aborted.
///
/// Without a runtime to run on, the orphan reaper deletes the pod at its
/// `expires-at` annotation. The persistent environment claim is not part of
/// it: it is meant to outlive the step.
pub(super) struct PodCleanup {
    pods: Api<Pod>,
    pod_name: String,
    configmap: Option<(Api<ConfigMap>, String)>,
    armed: bool,
}

impl PodCleanup {
    /// The pod `pod_name` and, when the prompt went through one, its ConfigMap.
    pub(super) fn new(
        pods: Api<Pod>,
        pod_name: String,
        configmap: Option<(Api<ConfigMap>, String)>,
    ) -> Self {
        Self {
            pods,
            pod_name,
            configmap,
            armed: true,
        }
    }

    /// Delete the pod and its ConfigMap now, best effort; disarms the drop.
    ///
    /// Disarmed only once the deletion returns: a step dropped in the middle
    /// sends it again from [`Drop`], which is harmless.
    pub(super) async fn delete(&mut self) {
        delete_all(&self.pods, &self.pod_name, self.configmap.as_ref()).await;
        self.armed = false;
    }
}

impl Drop for PodCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(handle) = Handle::try_current() else {
            warn!(
                pod = %self.pod_name,
                "pod not deleted: no runtime; the orphan reaper deletes it at expires-at"
            );
            return;
        };
        let pods = self.pods.clone();
        let pod_name = self.pod_name.clone();
        let configmap = self.configmap.clone();
        debug!(pod = %pod_name, "step dropped, deleting its pod");
        // Detached: the deletion outlives the dropped step.
        drop(handle.spawn(async move {
            delete_all(&pods, &pod_name, configmap.as_ref()).await;
        }));
    }
}

async fn delete_all(pods: &Api<Pod>, pod_name: &str, configmap: Option<&(Api<ConfigMap>, String)>) {
    if let Err(e) = pods.delete(pod_name, &DeleteParams::default()).await {
        warn!(
            pod = %pod_name,
            error = %e,
            "failed to delete the pod of a step; the orphan reaper deletes it at expires-at"
        );
    }
    if let Some((configmaps, name)) = configmap
        && let Err(e) = configmaps.delete(name, &DeleteParams::default()).await
    {
        warn!(
            pod = %pod_name,
            configmap = %name,
            error = %e,
            "failed to delete the prompt ConfigMap of a step"
        );
    }
}
