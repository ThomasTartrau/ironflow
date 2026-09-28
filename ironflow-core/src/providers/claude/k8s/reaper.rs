//! Orphan reaping for the pods, Jobs and prompt ConfigMaps ironflow creates.
//!
//! A worker that dies mid-step (OOM, eviction, hard shutdown) leaves its agent
//! pod and prompt ConfigMap behind, and so does one running a `PodRun` or a
//! `JobRun` of `ironflow-ops-k8s`. Every such object carries the
//! `app.kubernetes.io/managed-by=ironflow` label and the [`LABEL_EXPIRES_AT`]
//! annotation; the reaper deletes those whose expiry has passed, plus pods and
//! Jobs Kubernetes already killed for exceeding their `activeDeadlineSeconds`.
//!
//! The decision functions here are pure: [`reap_reason`], [`job_reap_reason`]
//! and [`configmap_expired`] never delete anything on doubt (missing or
//! unparseable annotation). [`reap_orphans`] applies them to a namespace.

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::Error as KubeError;
use kube::api::{Api, DeleteParams, ListParams};
use tracing::{info, warn};

use crate::error::AgentError;
use crate::provider::LABEL_EXPIRES_AT;

use super::common::{K8sClusterConfig, create_client};
use super::ephemeral::{MANAGED_SELECTOR, PROMPT_SELECTOR, now_unix};

/// Why the reaper deletes a pod or a Job.
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
    /// Kubernetes killed the object after its `activeDeadlineSeconds`.
    DeadlineExceeded,
    /// The object's [`LABEL_EXPIRES_AT`] annotation lies in the past.
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
    /// Number of Jobs deleted, their pods with them.
    pub jobs_deleted: usize,
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

/// Return `true` when a Job controls the object: the reaper removes it with
/// its Job, never on its own (the Job would start a new pod).
fn controlled_by_job(meta: &ObjectMeta) -> bool {
    meta.owner_references
        .iter()
        .flatten()
        .any(|owner| owner.kind == "Job" && owner.controller == Some(true))
}

/// Decide whether a pod must be reaped at `now_unix` (seconds).
///
/// Returns [`ReapReason::DeadlineExceeded`] for a `Failed` pod whose reason is
/// `DeadlineExceeded`, [`ReapReason::Expired`] for a pod past its
/// [`LABEL_EXPIRES_AT`] annotation whatever its phase (an `Unknown` pod on a
/// lost node included), and `None` otherwise, including when the annotation
/// is missing or unparseable, when the pod is already being deleted, or when
/// a Job controls the pod (see [`job_reap_reason`]).
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
    // A pod stuck terminating on a lost node would otherwise be deleted, and
    // counted, again on every pass.
    if pod.metadata.deletion_timestamp.is_some() || controlled_by_job(&pod.metadata) {
        return None;
    }
    let status = pod.status.as_ref();
    let phase = status.and_then(|s| s.phase.as_deref());
    let reason = status.and_then(|s| s.reason.as_deref());

    if phase == Some("Failed") && reason == Some("DeadlineExceeded") {
        return Some(ReapReason::DeadlineExceeded);
    }

    match expires_at(&pod.metadata) {
        Some(expiry) if expiry < now_unix => Some(ReapReason::Expired),
        _ => None,
    }
}

/// Decide whether a Job must be reaped at `now_unix` (seconds).
///
/// Returns [`ReapReason::DeadlineExceeded`] for a Job whose `Failed`
/// condition has the reason `DeadlineExceeded`, [`ReapReason::Expired`] for a
/// Job past its [`LABEL_EXPIRES_AT`] annotation, and `None` otherwise,
/// including when the annotation is missing or unparseable.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::job_reap_reason;
/// use k8s_openapi::api::batch::v1::Job;
///
/// assert_eq!(job_reap_reason(&Job::default(), 1_700_000_000), None);
/// ```
pub fn job_reap_reason(job: &Job, now_unix: u64) -> Option<ReapReason> {
    let conditions = job.status.as_ref().and_then(|s| s.conditions.as_ref());
    let deadline = conditions.into_iter().flatten().any(|c| {
        c.type_ == "Failed" && c.status == "True" && c.reason.as_deref() == Some("DeadlineExceeded")
    });
    if deadline {
        return Some(ReapReason::DeadlineExceeded);
    }
    match expires_at(&job.metadata) {
        Some(expiry) if expiry < now_unix => Some(ReapReason::Expired),
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

/// Delete the orphaned pods, Jobs and prompt ConfigMaps ironflow left in
/// `namespace`: agent pods, `PodRun` pods and `JobRun` Jobs, whatever their
/// component (`app.kubernetes.io/managed-by=ironflow`).
///
/// A pod or a Job is deleted when Kubernetes killed it for exceeding its
/// deadline, or when its [`LABEL_EXPIRES_AT`] annotation lies in the past
/// (see [`reap_reason`] and [`job_reap_reason`]); a Job goes with its pods.
/// Objects without a parseable annotation are never touched.
///
/// Needs no provider: a worker that only runs `PodRun` calls it directly.
/// Without the right to list Jobs, the Job pass is skipped with a warning.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] when the client cannot be built or
/// the pods, Jobs or ConfigMaps cannot be listed. A failed delete is logged
/// and not counted.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::claude::K8sClusterConfig;
/// use ironflow_core::providers::claude::k8s::reap_orphans;
///
/// # async fn example() -> Result<(), ironflow_core::error::AgentError> {
/// let report = reap_orphans(&K8sClusterConfig::Default, "ironflow-agents").await?;
/// println!("{} pods, {} jobs deleted", report.pods_deleted, report.jobs_deleted);
/// # Ok(())
/// # }
/// ```
pub async fn reap_orphans(
    cluster_config: &K8sClusterConfig,
    namespace: &str,
) -> Result<ReapReport, AgentError> {
    let client = create_client(cluster_config).await?;
    let now = now_unix()?;
    Ok(ReapReport {
        jobs_deleted: reap_jobs(&Api::namespaced(client.clone(), namespace), now).await?,
        pods_deleted: reap_pods(&Api::namespaced(client.clone(), namespace), now).await?,
        configmaps_deleted: reap_configmaps(&Api::namespaced(client, namespace), now).await?,
    })
}

fn list_error(what: &str, e: KubeError) -> AgentError {
    AgentError::ProcessFailed {
        exit_code: -1,
        stderr: format!("failed to list {what}: {e}"),
    }
}

async fn reap_pods(pods: &Api<Pod>, now: u64) -> Result<usize, AgentError> {
    let params = ListParams::default().labels(MANAGED_SELECTOR);
    let list = pods
        .list(&params)
        .await
        .map_err(|e| list_error("ironflow pods", e))?;
    let mut deleted = 0;
    for pod in &list.items {
        let (Some(name), Some(reason)) = (pod.metadata.name.as_deref(), reap_reason(pod, now))
        else {
            continue;
        };
        match pods.delete(name, &DeleteParams::default()).await {
            Ok(_) => {
                info!(pod = %name, ?reason, "reaped orphan pod");
                deleted += 1;
            }
            Err(e) => warn!(pod = %name, error = %e, "failed to reap orphan pod"),
        }
    }
    Ok(deleted)
}

async fn reap_jobs(jobs: &Api<Job>, now: u64) -> Result<usize, AgentError> {
    let params = ListParams::default().labels(MANAGED_SELECTOR);
    let list = match jobs.list(&params).await {
        Ok(list) => list,
        Err(KubeError::Api(e)) if e.code == 403 => {
            warn!(
                error = %e,
                "cannot list Jobs: orphan JobRun Jobs are not reaped; grant list and delete on jobs"
            );
            return Ok(0);
        }
        Err(e) => return Err(list_error("ironflow Jobs", e)),
    };
    let mut deleted = 0;
    for job in &list.items {
        let (Some(name), Some(reason)) = (job.metadata.name.as_deref(), job_reap_reason(job, now))
        else {
            continue;
        };
        match jobs.delete(name, &DeleteParams::background()).await {
            Ok(_) => {
                info!(job = %name, ?reason, "reaped orphan Job");
                deleted += 1;
            }
            Err(e) => warn!(job = %name, error = %e, "failed to reap orphan Job"),
        }
    }
    Ok(deleted)
}

async fn reap_configmaps(configmaps: &Api<ConfigMap>, now: u64) -> Result<usize, AgentError> {
    let params = ListParams::default().labels(PROMPT_SELECTOR);
    let listed = configmaps.list(&params).await;
    let list = listed.map_err(|e| list_error("prompt ConfigMaps", e))?;
    let mut deleted = 0;
    for cm in &list.items {
        let Some(name) = cm.metadata.name.as_deref() else {
            continue;
        };
        if !configmap_expired(cm, now) {
            continue;
        }
        match configmaps.delete(name, &DeleteParams::default()).await {
            Ok(_) => {
                info!(configmap = %name, "reaped orphan prompt ConfigMap");
                deleted += 1;
            }
            Err(e) => {
                warn!(configmap = %name, error = %e, "failed to reap orphan prompt ConfigMap");
            }
        }
    }
    Ok(deleted)
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

    fn job(failed_reason: Option<&str>, expires: Option<&str>) -> Job {
        let mut value: Value = json!({ "metadata": { "name": "migrate" }, "status": {} });
        if let Some(reason) = failed_reason {
            value["status"]["conditions"] =
                json!([{ "type": "Failed", "status": "True", "reason": reason }]);
        }
        if let Some(expires) = expires {
            value["metadata"]["annotations"] = json!({ LABEL_EXPIRES_AT: expires });
        }
        from_value(value).expect("valid job")
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
    fn k8s_reap_unknown_phase_past_expiry() {
        let p = pod("Unknown", None, Some(&past()));
        assert_eq!(reap_reason(&p, NOW), Some(ReapReason::Expired));
    }

    #[test]
    fn k8s_reap_unknown_phase_future_expiry_is_kept() {
        let p = pod("Unknown", None, Some(&future()));
        assert_eq!(reap_reason(&p, NOW), None);
    }

    #[test]
    fn k8s_reap_pod_already_terminating_is_left_alone() {
        let mut p = pod("Running", None, Some(&past()));
        let deleting = from_value(json!("2023-11-14T22:13:20Z")).expect("valid timestamp");
        p.metadata.deletion_timestamp = Some(deleting);
        assert_eq!(reap_reason(&p, NOW), None);
    }

    #[test]
    fn k8s_reap_pod_of_a_job_is_left_to_its_job() {
        let mut p = pod("Failed", Some("DeadlineExceeded"), Some(&past()));
        p.metadata.owner_references = Some(vec![
            from_value(json!({
                "apiVersion": "batch/v1",
                "kind": "Job",
                "name": "migrate",
                "uid": "uid-migrate",
                "controller": true,
            }))
            .expect("valid owner reference"),
        ]);
        assert_eq!(reap_reason(&p, NOW), None);
    }

    #[test]
    fn k8s_reap_job_reasons() {
        let deadline = job(Some("DeadlineExceeded"), Some(&future()));
        assert_eq!(
            job_reap_reason(&deadline, NOW),
            Some(ReapReason::DeadlineExceeded)
        );
        let expired = job(None, Some(&past()));
        assert_eq!(job_reap_reason(&expired, NOW), Some(ReapReason::Expired));
        assert_eq!(job_reap_reason(&job(None, Some(&future())), NOW), None);
        assert_eq!(job_reap_reason(&job(None, None), NOW), None);
        assert_eq!(job_reap_reason(&job(None, Some("soon")), NOW), None);
        let backoff = job(Some("BackoffLimitExceeded"), Some(&future()));
        assert_eq!(job_reap_reason(&backoff, NOW), None);
    }

    #[test]
    fn k8s_configmap_expired_true_and_false() {
        assert!(configmap_expired(&configmap(Some(&past())), NOW));
        assert!(!configmap_expired(&configmap(Some(&future())), NOW));
        assert!(!configmap_expired(&configmap(None), NOW));
        assert!(!configmap_expired(&configmap(Some("garbage")), NOW));
    }
}
