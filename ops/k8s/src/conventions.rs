//! Labels and annotation carried by every pod and Job this crate creates.
//!
//! [`PodRun`](crate::pod_run::PodRun) and [`JobRun`](crate::job_run::JobRun)
//! follow the conventions of the ironflow agent pods, so the orphan reaper of
//! `ironflow-core` (`reap_orphans`) and the run cleanup of the K8s provider
//! find them after the worker that created them died:
//!
//! * [`LABEL_MANAGED_BY`] = [`MANAGED_BY_IRONFLOW`] and [`LABEL_COMPONENT`],
//!   set by ironflow and refused from the caller;
//! * [`LABEL_EXPIRES_AT`]: creation + timeout + [`DEFAULT_EXPIRY_MARGIN`].
//!
//! Tag the object with [`LABEL_RUN_ID`](ironflow_core::provider::LABEL_RUN_ID)
//! yourself so that a retry of the run deletes it.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::Utc;
use ironflow_core::provider::{
    LABEL_COMPONENT, LABEL_EXPIRES_AT, LABEL_MANAGED_BY, MANAGED_BY_IRONFLOW,
    assert_pod_label_allowed,
};

#[cfg(test)]
mod tests;

/// Time added to the timeout of a [`PodRun`](crate::pod_run::PodRun) or
/// [`JobRun`](crate::job_run::JobRun) for its [`LABEL_EXPIRES_AT`]
/// annotation, unless set with `expiry_margin`.
pub const DEFAULT_EXPIRY_MARGIN: Duration = Duration::from_secs(60);

/// [`LABEL_COMPONENT`] of the pods created by [`PodRun`](crate::pod_run::PodRun).
pub const POD_RUN_COMPONENT: &str = "pod-run";

/// [`LABEL_COMPONENT`] of the Jobs and pods created by
/// [`JobRun`](crate::job_run::JobRun).
pub const JOB_RUN_COMPONENT: &str = "job-run";

/// The caller's labels plus [`LABEL_MANAGED_BY`] and `component`.
pub(crate) fn ironflow_labels(
    caller: &BTreeMap<String, String>,
    component: &str,
) -> BTreeMap<String, String> {
    let mut labels = caller.clone();
    labels.insert(
        LABEL_MANAGED_BY.to_string(),
        MANAGED_BY_IRONFLOW.to_string(),
    );
    labels.insert(LABEL_COMPONENT.to_string(), component.to_string());
    labels
}

/// The [`LABEL_EXPIRES_AT`] annotation of an object that lives `lifetime`
/// from now.
pub(crate) fn expiry_annotation(lifetime: Duration) -> BTreeMap<String, String> {
    let lifetime = i64::try_from(lifetime.as_secs()).unwrap_or(i64::MAX);
    let expires_at = Utc::now().timestamp().saturating_add(lifetime);
    BTreeMap::from([(LABEL_EXPIRES_AT.to_string(), expires_at.to_string())])
}

/// Refuse a map of caller labels holding a reserved key.
///
/// # Panics
///
/// Panics on the first key [`assert_pod_label_allowed`] refuses.
pub(crate) fn assert_labels_allowed(labels: &BTreeMap<String, String>) {
    for key in labels.keys() {
        assert_pod_label_allowed(key);
    }
}
