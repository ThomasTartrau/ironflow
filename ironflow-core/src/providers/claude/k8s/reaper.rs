//! Orphan reaping for pods and prompt ConfigMaps of the ephemeral provider.
//!
//! A worker that dies mid-step (OOM, eviction, hard shutdown) leaves its agent
//! pod and prompt ConfigMap behind. Every object the ephemeral provider creates
//! carries the [`LABEL_EXPIRES_AT`] annotation; the reaper deletes those whose
//! expiry has passed, plus pods Kubernetes already killed for exceeding their
//! `activeDeadlineSeconds`.
//!
//! The decision functions here are pure: [`reap_reason`] and
//! [`configmap_expired`] never delete anything on doubt (missing or
//! unparseable annotation). The pass that applies them to a namespace backs
//! [`K8sEphemeralProvider::reap_orphans`](super::K8sEphemeralProvider::reap_orphans).

use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, DeleteParams, ListParams};
use tracing::{info, warn};

use crate::error::AgentError;

use super::common::{K8sClusterConfig, LABEL_EXPIRES_AT, create_client};
use super::ephemeral::{PROMPT_SELECTOR, RUNNER_SELECTOR, now_unix};

/// Why the reaper deletes a pod.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::ReapReason;
///
/// assert_ne!(ReapReason::DeadlineExceeded, ReapReason::Expired);
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReapReason {
    /// Kubernetes killed the pod after its `activeDeadlineSeconds`.
    DeadlineExceeded,
    /// The pod's [`LABEL_EXPIRES_AT`] annotation lies in the past.
    Expired,
}

/// Outcome of one reaping pass.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::ReapReport;
///
/// let report = ReapReport::default();
/// assert_eq!(report.pods_deleted, 0);
/// ```
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ReapReport {
    /// Number of pods deleted.
    pub pods_deleted: usize,
    /// Number of prompt ConfigMaps deleted.
    pub configmaps_deleted: usize,
}

/// Read the [`LABEL_EXPIRES_AT`] annotation as unix seconds.
fn expires_at(meta: &ObjectMeta) -> Option<u64> {
    meta.annotations
        .as_ref()?
        .get(LABEL_EXPIRES_AT)?
        .parse()
        .ok()
}

/// Decide whether a pod must be reaped at `now_unix` (seconds).
///
/// Returns [`ReapReason::DeadlineExceeded`] for a `Failed` pod whose reason is
/// `DeadlineExceeded`, [`ReapReason::Expired`] for a pod past its
/// [`LABEL_EXPIRES_AT`] annotation in a known phase, and `None` otherwise,
/// including when the annotation is missing or unparseable.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::reap_reason;
/// use k8s_openapi::api::core::v1::Pod;
///
/// let pod = Pod::default();
/// assert_eq!(reap_reason(&pod, 1_700_000_000), None);
/// ```
pub fn reap_reason(pod: &Pod, now_unix: u64) -> Option<ReapReason> {
    let status = pod.status.as_ref();
    let phase = status.and_then(|s| s.phase.as_deref());
    let reason = status.and_then(|s| s.reason.as_deref());

    if phase == Some("Failed") && reason == Some("DeadlineExceeded") {
        return Some(ReapReason::DeadlineExceeded);
    }

    let known_phase = matches!(phase, Some("Running" | "Pending" | "Succeeded" | "Failed"));
    match expires_at(&pod.metadata) {
        Some(expiry) if known_phase && expiry < now_unix => Some(ReapReason::Expired),
        _ => None,
    }
}

/// Return `true` when a prompt ConfigMap is past its [`LABEL_EXPIRES_AT`]
/// annotation at `now_unix` (seconds). A missing or unparseable annotation
/// returns `false`.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::configmap_expired;
/// use k8s_openapi::api::core::v1::ConfigMap;
///
/// assert!(!configmap_expired(&ConfigMap::default(), 1_700_000_000));
/// ```
pub fn configmap_expired(cm: &ConfigMap, now_unix: u64) -> bool {
    expires_at(&cm.metadata).is_some_and(|expiry| expiry < now_unix)
}

/// One reaping pass over `namespace`: see
/// [`K8sEphemeralProvider::reap_orphans`](super::K8sEphemeralProvider::reap_orphans).
///
/// Takes only what a pass needs, so the periodic reaper task does not keep a
/// whole provider alive.
pub(super) async fn reap_namespace(
    cluster_config: &K8sClusterConfig,
    namespace: &str,
) -> Result<ReapReport, AgentError> {
    let client = create_client(cluster_config).await?;
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let configmaps: Api<ConfigMap> = Api::namespaced(client, namespace);
    let now = now_unix()?;
    let mut report = ReapReport::default();

    let pod_params = ListParams::default().labels(RUNNER_SELECTOR);
    let listed = pods.list(&pod_params).await;
    let pod_list = listed.map_err(|e| AgentError::ProcessFailed {
        exit_code: -1,
        stderr: format!("failed to list agent pods: {e}"),
    })?;
    for pod in &pod_list.items {
        let Some(name) = pod.metadata.name.as_deref() else {
            continue;
        };
        let Some(reason) = reap_reason(pod, now) else {
            continue;
        };
        match pods.delete(name, &DeleteParams::default()).await {
            Ok(_) => {
                info!(pod = %name, ?reason, "reaped orphan agent pod");
                report.pods_deleted += 1;
            }
            Err(e) => warn!(pod = %name, error = %e, "failed to reap orphan agent pod"),
        }
    }

    let cm_params = ListParams::default().labels(PROMPT_SELECTOR);
    let listed = configmaps.list(&cm_params).await;
    let cm_list = listed.map_err(|e| AgentError::ProcessFailed {
        exit_code: -1,
        stderr: format!("failed to list prompt ConfigMaps: {e}"),
    })?;
    for cm in &cm_list.items {
        let Some(name) = cm.metadata.name.as_deref() else {
            continue;
        };
        if !configmap_expired(cm, now) {
            continue;
        }
        match configmaps.delete(name, &DeleteParams::default()).await {
            Ok(_) => {
                info!(configmap = %name, "reaped orphan prompt ConfigMap");
                report.configmaps_deleted += 1;
            }
            Err(e) => {
                warn!(configmap = %name, error = %e, "failed to reap orphan prompt ConfigMap");
            }
        }
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, from_value, json};

    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn pod(phase: &str, reason: Option<&str>, expires: Option<&str>) -> Pod {
        let mut value: Value = json!({
            "metadata": { "name": "claude-code-1" },
            "spec": { "containers": [] },
            "status": { "phase": phase }
        });
        if let Some(reason) = reason {
            value["status"]["reason"] = json!(reason);
        }
        if let Some(expires) = expires {
            value["metadata"]["annotations"] = json!({ LABEL_EXPIRES_AT: expires });
        }
        from_value(value).expect("valid pod")
    }

    fn configmap(expires: Option<&str>) -> ConfigMap {
        let mut value: Value = json!({ "metadata": { "name": "claude-code-1-prompt" } });
        if let Some(expires) = expires {
            value["metadata"]["annotations"] = json!({ LABEL_EXPIRES_AT: expires });
        }
        from_value(value).expect("valid configmap")
    }

    fn past() -> String {
        (NOW - 10).to_string()
    }

    fn future() -> String {
        (NOW + 10).to_string()
    }

    #[test]
    fn k8s_reap_failed_deadline_exceeded() {
        let p = pod("Failed", Some("DeadlineExceeded"), None);
        assert_eq!(reap_reason(&p, NOW), Some(ReapReason::DeadlineExceeded));
    }

    #[test]
    fn k8s_reap_running_past_expiry() {
        let p = pod("Running", None, Some(&past()));
        assert_eq!(reap_reason(&p, NOW), Some(ReapReason::Expired));
    }

    #[test]
    fn k8s_reap_running_future_expiry_is_kept() {
        let p = pod("Running", None, Some(&future()));
        assert_eq!(reap_reason(&p, NOW), None);
    }

    #[test]
    fn k8s_reap_running_without_annotation_is_kept() {
        let p = pod("Running", None, None);
        assert_eq!(reap_reason(&p, NOW), None);
    }

    #[test]
    fn k8s_reap_garbage_annotation_is_kept() {
        let p = pod("Running", None, Some("tomorrow"));
        assert_eq!(reap_reason(&p, NOW), None);
    }

    #[test]
    fn k8s_reap_succeeded_past_expiry() {
        let p = pod("Succeeded", None, Some(&past()));
        assert_eq!(reap_reason(&p, NOW), Some(ReapReason::Expired));
    }

    #[test]
    fn k8s_reap_pending_past_expiry() {
        let p = pod("Pending", None, Some(&past()));
        assert_eq!(reap_reason(&p, NOW), Some(ReapReason::Expired));
    }

    #[test]
    fn k8s_reap_failed_other_reason_future_expiry_is_kept() {
        let p = pod("Failed", Some("Evicted"), Some(&future()));
        assert_eq!(reap_reason(&p, NOW), None);
    }

    #[test]
    fn k8s_reap_unknown_phase_past_expiry_is_kept() {
        let p = pod("Unknown", None, Some(&past()));
        assert_eq!(reap_reason(&p, NOW), None);
    }

    #[test]
    fn k8s_configmap_expired_true_and_false() {
        assert!(configmap_expired(&configmap(Some(&past())), NOW));
        assert!(!configmap_expired(&configmap(Some(&future())), NOW));
        assert!(!configmap_expired(&configmap(None), NOW));
        assert!(!configmap_expired(&configmap(Some("garbage")), NOW));
    }
}
