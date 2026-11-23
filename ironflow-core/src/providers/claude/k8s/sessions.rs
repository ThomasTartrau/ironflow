//! Claude Code sessions volume of a sandboxed agent pod.
//!
//! Two sources, the step winning: the provider's
//! [`sessions_volume`](super::ephemeral::K8sEphemeralProvider::sessions_volume),
//! a whole claim shared by every pod, and the step's
//! [`AgentConfig::sessions_pvc`](crate::provider::AgentConfig::sessions_pvc),
//! `<dir>/<run id>` of a claim, so a pod only sees the sessions of its run.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::error::AgentError;
use crate::provider::{LABEL_RUN_ID, PodVolumeSource, SessionsVolume};

use super::common::{
    PodConfig, SESSIONS_MOUNT_PATH, hardening_error as sessions_error, mount_json,
};

/// Name of the pod volume declared for the sessions claim.
const SESSIONS_VOLUME_NAME: &str = "ironflow-sessions";

/// Claim and optional `subPath` mounted on [`SESSIONS_MOUNT_PATH`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SessionsMount {
    pub(super) claim_name: String,
    pub(super) sub_path: Option<String>,
}

/// Pick the sessions mount of a pod: the step's sessions under the run id
/// of the pod labels, else the provider's whole claim.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] when the step sets sessions on a
/// provider that is not sandboxed, when the pod has no run id label, or when
/// the run id cannot name a directory.
pub(super) fn resolve_sessions(
    provider_claim: Option<&str>,
    sandboxed: bool,
    step: Option<&SessionsVolume>,
    labels: &BTreeMap<String, String>,
) -> Result<Option<SessionsMount>, AgentError> {
    let Some(step) = step else {
        return Ok(provider_claim.map(|claim| SessionsMount {
            claim_name: claim.to_string(),
            sub_path: None,
        }));
    };
    if !sandboxed {
        return Err(sessions_error(
            "sessions_pvc requires a provider built with K8sEphemeralProvider::sandboxed"
                .to_string(),
        ));
    }
    let run_id = labels.get(LABEL_RUN_ID).ok_or_else(|| {
        sessions_error(format!(
            "sessions_pvc needs the '{LABEL_RUN_ID}' pod label, set by the engine on every agent step"
        ))
    })?;
    let sub_path = step
        .run_sub_path(run_id)
        .map_err(|reason| sessions_error(format!("sessions_pvc: {reason}")))?;
    Ok(Some(SessionsMount {
        claim_name: step.claim_name.clone(),
        sub_path: Some(sub_path),
    }))
}

/// Refuse another mount of the sessions claim that shows the sessions of
/// every run: the claim root, or a directory holding the runs' directories.
///
/// Only checked for a sessions mount with a `subPath`: the provider's whole
/// claim is shared by design.
///
/// # Errors
///
/// Returns the reason as a message when such a mount exists.
pub(super) fn check_sessions_isolation(config: &PodConfig<'_>) -> Result<(), String> {
    let hardening = &config.hardening;
    let (Some(claim), Some(sub_path)) = (hardening.sessions_claim, hardening.sessions_sub_path)
    else {
        return Ok(());
    };
    // The directory holding one sub-directory per run.
    let runs_dir = sub_path.rsplit_once('/').map_or("", |(dir, _)| dir);
    let exposes_runs = |other: Option<&str>| match other {
        None => true,
        Some(other) => runs_dir == other || runs_dir.starts_with(&format!("{other}/")),
    };

    let provider_mounts = config
        .pvc_volumes
        .iter()
        .filter(|(name, _)| name == claim)
        .map(|(_, mount)| mount.as_str());
    let step_mounts = hardening
        .step_pvc_volumes
        .iter()
        .filter(|v| v.claim_name == claim && exposes_runs(v.sub_path.as_deref()))
        .map(|v| v.mount_path.as_str());
    let read_only_mounts = hardening
        .read_only_volumes
        .iter()
        .filter(|v| {
            matches!(&v.source, PodVolumeSource::PersistentVolumeClaim { claim_name } if claim_name == claim)
                && exposes_runs(v.sub_path.as_deref())
        })
        .map(|v| v.mount_path.as_str());

    match provider_mounts
        .chain(step_mounts)
        .chain(read_only_mounts)
        .next()
    {
        Some(mount) => Err(format!(
            "claim '{claim}' holds the sessions of every run in '{runs_dir}' and is also mounted at \
             '{mount}', which shows them: mount that claim on a sub_path outside '{runs_dir}'"
        )),
        None => Ok(()),
    }
}

/// Volume (when the pod does not declare the claim yet) and mount JSON of
/// the sessions claim.
///
/// Without a `subPath` the claim always gets its own volume, as before step
/// sessions existed. With one, a claim the pod already declares (in
/// `claim_volumes`) reuses that volume.
pub(super) fn sessions_volume_json(
    claim: &str,
    sub_path: Option<&str>,
    claim_volumes: &BTreeMap<&str, String>,
) -> (Option<Value>, Value) {
    let reused = sub_path.and_then(|_| claim_volumes.get(claim));
    let (volume, name) = match reused {
        Some(name) => (None, name.as_str()),
        None => (
            Some(json!({
                "name": SESSIONS_VOLUME_NAME,
                "persistentVolumeClaim": { "claimName": claim }
            })),
            SESSIONS_VOLUME_NAME,
        ),
    };
    (
        volume,
        mount_json(name, SESSIONS_MOUNT_PATH, sub_path, false),
    )
}

#[cfg(test)]
mod tests;
