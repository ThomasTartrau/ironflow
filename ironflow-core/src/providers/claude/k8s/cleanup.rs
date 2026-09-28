//! Deletion of what a previous attempt left running, before new work starts.
//!
//! Two scopes share [`delete_and_wait`]:
//!
//! * a step ([`step_selection`]): before the ephemeral provider starts the
//!   pod of a step again, it deletes the pods of the previous attempt of the
//!   same step of the same run;
//! * a run ([`release_run`]): before the engine executes a run again, every
//!   pod, Job and prompt ConfigMap carrying the run id, or the run as root of
//!   a sub-workflow, is deleted.
//!
//! In both cases the pods must be gone before the call returns: two agents
//! never work side by side on the same state.

use std::collections::BTreeMap;
use std::time::Duration;

use futures_util::future::join_all;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use kube::api::{Api, DeleteParams, ListParams};
use kube::runtime::wait::{await_condition, conditions};
use kube::{Client, Error as KubeError};
use tokio::time;
use tracing::{info, warn};

use crate::error::AgentError;
use crate::provider::{LABEL_ROOT_RUN_ID, LABEL_RUN_ID, LABEL_STEP, sanitize_label_value};

use super::common::{K8sClusterConfig, create_client};
use super::ephemeral::{MANAGED_SELECTOR, PROMPT_SELECTOR, RUNNER_SELECTOR};

/// The label selectors of one cleanup, and what it is about for messages.
pub(super) struct Selection {
    pods: Vec<String>,
    jobs: Vec<String>,
    configmaps: Vec<String>,
    what: String,
}

/// The agent pods and prompt ConfigMaps of a previous attempt of `step` in
/// the run `run_id` (both label values as written on the pod).
pub(super) fn step_selection(run_id: &str, step: &str) -> Selection {
    let scope = format!("{LABEL_RUN_ID}={run_id},{LABEL_STEP}={step}");
    Selection {
        pods: vec![format!("{RUNNER_SELECTOR},{scope}")],
        jobs: Vec::new(),
        configmaps: vec![format!("{PROMPT_SELECTOR},{scope}")],
        what: format!("the previous attempt of step {step} of run {run_id}"),
    }
}

/// Every pod, Job and prompt ConfigMap of the run `run_id`, or of a
/// sub-workflow whose root is `run_id`.
fn run_selection(run_id: &str) -> Selection {
    let id = sanitize_label_value(run_id);
    let scopes = [
        format!("{LABEL_RUN_ID}={id}"),
        format!("{LABEL_ROOT_RUN_ID}={id}"),
    ];
    let managed = scopes.iter().map(|s| format!("{MANAGED_SELECTOR},{s}"));
    Selection {
        pods: managed.clone().collect(),
        jobs: managed.collect(),
        configmaps: scopes
            .iter()
            .map(|s| format!("{PROMPT_SELECTOR},{s}"))
            .collect(),
        what: format!("run {run_id}"),
    }
}

/// Delete every pod, Job and prompt ConfigMap of the run `run_id` in
/// `namespace`, and wait up to `limit` until the pods are gone.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] when the client cannot be built,
/// the objects cannot be listed or deleted, or a pod is still there after
/// `limit`.
pub(super) async fn release_run(
    cluster_config: &K8sClusterConfig,
    namespace: &str,
    run_id: &str,
    limit: Duration,
) -> Result<(), AgentError> {
    let client = create_client(cluster_config).await?;
    let deleted = delete_and_wait(&client, namespace, &run_selection(run_id), limit).await?;
    if deleted > 0 {
        info!(run_id = %run_id, pods_deleted = deleted, "released the pods of a previous execution");
    }
    Ok(())
}

fn failure(stderr: String) -> AgentError {
    AgentError::ProcessFailed {
        exit_code: -1,
        stderr,
    }
}

/// Delete what `selection` matches: Jobs first (with their pods, or the Job
/// would start a new one), then pods, and wait up to `limit` until the pods
/// are gone; prompt ConfigMaps last, best effort.
///
/// Returns the number of pods deleted.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] when the pods or Jobs cannot be
/// listed or deleted, or a pod is still there after `limit`. A 403 on the
/// Jobs is logged and skipped: without that right, no Job can be listed.
pub(super) async fn delete_and_wait(
    client: &Client,
    namespace: &str,
    selection: &Selection,
    limit: Duration,
) -> Result<usize, AgentError> {
    let what = &selection.what;
    delete_jobs(&Api::namespaced(client.clone(), namespace), selection).await?;

    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    // A pod may match several selectors: keyed by uid, deleted once.
    let mut listed: BTreeMap<String, String> = BTreeMap::new();
    for selector in &selection.pods {
        let list = pods.list(&ListParams::default().labels(selector)).await;
        let list = list.map_err(|e| failure(format!("failed to list pods of {what}: {e}")))?;
        for pod in list.items {
            if let (Some(name), Some(uid)) = (pod.metadata.name, pod.metadata.uid) {
                listed.insert(uid, name);
            }
        }
    }

    let mut deleted: Vec<(String, String)> = Vec::new();
    for (uid, name) in listed {
        match pods.delete(&name, &DeleteParams::default()).await {
            Ok(_) => deleted.push((name, uid)),
            // Already gone between the list and the delete.
            Err(KubeError::Api(e)) if e.code == 404 => {}
            Err(e) => {
                return Err(failure(format!(
                    "failed to delete pod '{name}' of {what}: {e}"
                )));
            }
        }
    }

    let waits = deleted
        .iter()
        .map(|(name, uid)| await_condition(pods.clone(), name, conditions::is_deleted(uid)));
    let waited = time::timeout(limit, join_all(waits)).await;
    let results = waited.map_err(|e| {
        failure(format!(
            "pods of {what} still terminating after {limit:?}: {e}"
        ))
    })?;
    for result in results {
        result.map_err(|e| failure(format!("failed waiting for the pods of {what}: {e}")))?;
    }

    delete_configmaps(&Api::namespaced(client.clone(), namespace), selection).await;
    Ok(deleted.len())
}

async fn delete_jobs(jobs: &Api<Job>, selection: &Selection) -> Result<(), AgentError> {
    let what = &selection.what;
    for selector in &selection.jobs {
        let list = match jobs.list(&ListParams::default().labels(selector)).await {
            Ok(list) => list,
            Err(KubeError::Api(e)) if e.code == 403 => {
                warn!(error = %e, "cannot list Jobs: JobRun Jobs of {what} are not deleted; grant list and delete on jobs");
                continue;
            }
            Err(e) => return Err(failure(format!("failed to list Jobs of {what}: {e}"))),
        };
        for name in list.items.into_iter().filter_map(|job| job.metadata.name) {
            match jobs.delete(&name, &DeleteParams::background()).await {
                Ok(_) => {}
                Err(KubeError::Api(e)) if e.code == 404 => {}
                Err(e) => {
                    return Err(failure(format!(
                        "failed to delete Job '{name}' of {what}: {e}"
                    )));
                }
            }
        }
    }
    Ok(())
}

/// Delete the prompt ConfigMaps: best effort, a leftover is only data and
/// the reaper removes it once expired.
async fn delete_configmaps(configmaps: &Api<ConfigMap>, selection: &Selection) {
    let mut names: Vec<String> = Vec::new();
    for selector in &selection.configmaps {
        match configmaps
            .list(&ListParams::default().labels(selector))
            .await
        {
            Ok(list) => names.extend(list.items.into_iter().filter_map(|cm| cm.metadata.name)),
            Err(e) => warn!(error = %e, "failed to list prompt ConfigMaps of {}", selection.what),
        }
    }
    names.sort();
    names.dedup();
    for name in names {
        if let Err(e) = configmaps.delete(&name, &DeleteParams::default()).await {
            warn!(configmap = %name, error = %e, "failed to delete prompt ConfigMap");
        }
    }
}
