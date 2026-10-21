//! How a [`PodRun`](super::PodRun) pod ended: phase, exit code and reason.

use k8s_openapi::api::core::v1::PodStatus;

#[cfg(test)]
mod tests;

/// The end of a pod that reached a terminal phase.
pub(crate) struct Termination {
    /// `"Succeeded"` or `"Failed"`.
    pub(crate) phase: String,
    /// `terminated.exitCode` of the run's container, when it terminated.
    pub(crate) exit_code: Option<i32>,
    /// The pod's `status.reason`, else the container's `terminated.reason`.
    pub(crate) reason: Option<String>,
}

impl Termination {
    /// Read the end of `status`, `None` while the pod is not terminal.
    ///
    /// The container is looked up by `container` name, not taken at index 0:
    /// the order of `containerStatuses` is not guaranteed, and a sidecar
    /// injected by an admission webhook can come first. The pod's own reason
    /// wins over the container's: a pod killed by `activeDeadlineSeconds` or
    /// evicted says so on the pod, while its container may show `Error`, or
    /// still `running`.
    pub(crate) fn of(status: &PodStatus, container: &str) -> Option<Self> {
        let phase = status.phase.as_deref()?;
        if phase != "Succeeded" && phase != "Failed" {
            return None;
        }
        let terminated = status
            .container_statuses
            .iter()
            .flatten()
            .find(|s| s.name == container)
            .and_then(|s| s.state.as_ref())
            .and_then(|state| state.terminated.as_ref());
        Some(Self {
            phase: phase.to_string(),
            exit_code: terminated.map(|t| t.exit_code),
            reason: status
                .reason
                .clone()
                .or_else(|| terminated.and_then(|t| t.reason.clone())),
        })
    }
}
