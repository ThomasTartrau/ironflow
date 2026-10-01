//! Per-step pod settings for the Kubernetes ephemeral provider.
//!
//! [`PodSettings`] travels inside [`AgentConfig`](super::AgentConfig) so a step
//! can ask for secrets, a service account, read-only volumes or a managed
//! settings preset. Only `K8sEphemeralProvider` reads it; every other provider
//! ignores it. The types live here without a feature gate because the engine
//! and the workflow author set them regardless of the transport.

use serde::{Deserialize, Serialize};

/// Pod label carrying the id of the run that created the pod.
///
/// Stamped by the engine on every agent step. Together with [`LABEL_STEP`] it
/// lets the provider find and delete the pods of a previous attempt of the
/// same step before starting a retry.
pub const LABEL_RUN_ID: &str = "ironflow.io/run-id";

/// Pod label carrying the sanitized name of the step that created the pod.
///
/// The value goes through [`sanitize_label_value`].
pub const LABEL_STEP: &str = "ironflow.io/step";

/// Pod label carrying the id of the top-level run of the pod's run.
///
/// Equal to [`LABEL_RUN_ID`] for a run started on its own; a sub-workflow's
/// child run carries its parent's root. Stamped by the engine on every agent
/// step, so a retry of the top-level run finds the pods its children left
/// behind.
pub const LABEL_ROOT_RUN_ID: &str = "ironflow.io/root-run-id";

/// Pod label selecting the network egress profile of the pod.
///
/// Network policies select agent pods on this label to open egress to the
/// hosts of a profile (for instance `gitlab`).
pub const LABEL_EGRESS_PROFILE: &str = "ironflow.io/egress-profile";

/// Label naming the tool that manages an object, set to
/// [`MANAGED_BY_IRONFLOW`] on every pod, Job and ConfigMap ironflow creates.
///
/// Reserved: see [`is_reserved_pod_label`].
pub const LABEL_MANAGED_BY: &str = "app.kubernetes.io/managed-by";

/// Value of [`LABEL_MANAGED_BY`] on every object ironflow creates. The orphan
/// reaper and the run cleanup select on it.
pub const MANAGED_BY_IRONFLOW: &str = "ironflow";

/// Label naming what created the object inside ironflow: `claude-runner`,
/// `prompt-data`, `pod-run` or `job-run`.
///
/// Reserved: see [`is_reserved_pod_label`].
pub const LABEL_COMPONENT: &str = "app.kubernetes.io/component";

/// Annotation holding the unix time (seconds) after which an ironflow pod,
/// Job or prompt ConfigMap is considered orphaned and may be reaped.
pub const LABEL_EXPIRES_AT: &str = "ironflow.io/expires-at";

/// Return `true` for a label ironflow sets itself on every object it
/// creates ([`LABEL_MANAGED_BY`], [`LABEL_COMPONENT`]): a caller cannot set
/// it, since the orphan reaper and the run cleanup select on it.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::{LABEL_COMPONENT, LABEL_RUN_ID, is_reserved_pod_label};
///
/// assert!(is_reserved_pod_label(LABEL_COMPONENT));
/// assert!(!is_reserved_pod_label(LABEL_RUN_ID));
/// ```
pub fn is_reserved_pod_label(key: &str) -> bool {
    key == LABEL_MANAGED_BY || key == LABEL_COMPONENT
}

/// Refuse a reserved label passed to a pod builder.
///
/// Called by every builder that takes a label from its caller: the K8s
/// providers, `PodRun` and `JobRun` of `ironflow-ops-k8s`.
///
/// # Panics
///
/// Panics when [`is_reserved_pod_label`] returns `true` for `key`.
///
/// # Examples
///
/// ```should_panic
/// use ironflow_core::provider::{LABEL_COMPONENT, assert_pod_label_allowed};
///
/// assert_pod_label_allowed(LABEL_COMPONENT);
/// ```
pub fn assert_pod_label_allowed(key: &str) {
    assert!(
        !is_reserved_pod_label(key),
        "pod label '{key}' is reserved: ironflow sets it on every object it creates"
    );
}

/// Maximum length of a Kubernetes label value, in bytes.
const LABEL_VALUE_MAX: usize = 63;

/// Length of the `-xxxxxxxx` hash suffix appended to altered values.
const HASH_SUFFIX_LEN: usize = 9;

/// Environment variable read from a Kubernetes Secret.
///
/// Rendered as `valueFrom.secretKeyRef`: the value never enters the pod spec.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::SecretEnvVar;
///
/// let var = SecretEnvVar {
///     name: "CLAUDE_CODE_OAUTH_TOKEN".to_string(),
///     secret: "claude-oauth".to_string(),
///     key: "token".to_string(),
/// };
/// assert_eq!(var.secret, "claude-oauth");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct SecretEnvVar {
    /// Name of the environment variable inside the container.
    pub name: String,
    /// Name of the Secret in the pod's namespace.
    pub secret: String,
    /// Key inside the Secret.
    pub key: String,
}

/// Source of a [`ReadOnlyVolume`].
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::PodVolumeSource;
///
/// let source = PodVolumeSource::PersistentVolumeClaim {
///     claim_name: "repos".to_string(),
/// };
/// assert!(matches!(source, PodVolumeSource::PersistentVolumeClaim { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PodVolumeSource {
    /// An existing PersistentVolumeClaim in the pod's namespace.
    PersistentVolumeClaim {
        /// Name of the claim.
        claim_name: String,
    },
    /// A directory on the node.
    HostPath {
        /// Absolute path of the directory on the node.
        path: String,
    },
    /// An existing ConfigMap in the pod's namespace.
    ConfigMap {
        /// Name of the ConfigMap.
        name: String,
    },
}

/// A volume mounted read-only into the agent container.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::{PodVolumeSource, ReadOnlyVolume};
///
/// let volume = ReadOnlyVolume {
///     source: PodVolumeSource::ConfigMap { name: "prompts".to_string() },
///     mount_path: "/data/prompts".to_string(),
///     sub_path: None,
/// };
/// assert_eq!(volume.mount_path, "/data/prompts");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadOnlyVolume {
    /// Where the data comes from.
    pub source: PodVolumeSource,
    /// Absolute mount path inside the container.
    pub mount_path: String,
    /// Optional sub-path of the volume to mount instead of its root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_path: Option<String>,
}

/// A PersistentVolumeClaim mounted into the agent container, read-write by
/// default.
///
/// Unlike [`ReadOnlyVolume`], the mount can be read-write and carries an
/// optional `sub_path`. Several mounts may share one claim.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::PvcVolume;
///
/// let volume = PvcVolume {
///     claim_name: "repos".to_string(),
///     mount_path: "/data/repos".to_string(),
///     sub_path: Some("team-a".to_string()),
///     read_only: true,
/// };
/// assert_eq!(volume.claim_name, "repos");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PvcVolume {
    /// Name of the claim in the pod's namespace.
    pub claim_name: String,
    /// Absolute mount path inside the container.
    pub mount_path: String,
    /// Optional sub-path of the volume to mount instead of its root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_path: Option<String>,
    /// Mount the volume read-only.
    #[serde(default)]
    pub read_only: bool,
}

/// Check the structure of a PVC `subPath`.
///
/// Refuses an empty value, a leading `/`, an empty segment (a trailing `/` or
/// a `//`) and any `.` or `..` segment. Characters are not restricted: a
/// volume's directories may contain spaces.
///
/// # Errors
///
/// Returns the reason as a message when `sub_path` is refused.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::validate_pvc_sub_path;
///
/// assert!(validate_pvc_sub_path("team a/repos").is_ok());
/// assert!(validate_pvc_sub_path("../x").is_err());
/// assert!(validate_pvc_sub_path("/abs").is_err());
/// assert!(validate_pvc_sub_path("a//b").is_err());
/// ```
pub fn validate_pvc_sub_path(sub_path: &str) -> Result<(), String> {
    if sub_path.is_empty() {
        return Err("sub_path must not be empty".to_string());
    }
    if sub_path.starts_with('/') {
        return Err(format!("sub_path '{sub_path}' must be relative"));
    }
    for segment in sub_path.split('/') {
        match segment {
            "" => return Err(format!("sub_path '{sub_path}' has an empty segment")),
            "." | ".." => {
                return Err(format!(
                    "sub_path '{sub_path}' must not contain '.' or '..' segments"
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// Pod-level settings a step asks for (K8s ephemeral provider only).
///
/// Merged with the provider's own settings when the pod is built: the step
/// wins on conflicts (same env var name, service account, managed settings).
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::PodSettings;
///
/// let settings = PodSettings::default();
/// assert!(settings.is_empty());
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PodSettings {
    /// Environment variables read from Kubernetes Secrets.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub secret_env: Vec<SecretEnvVar>,
    /// Service account of the pod. Overrides the provider's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service_account: Option<String>,
    /// Volumes mounted read-only, after the provider's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub read_only_volumes: Vec<ReadOnlyVolume>,
    /// Name of a managed-settings preset registered on the provider.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_settings: Option<String>,
    /// PVC mounts of the step, merged after the provider's volumes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pvc_volumes: Vec<PvcVolume>,
    /// Drop the provider's `volume` and `pvc_volume` mounts for this step.
    #[serde(default, skip_serializing_if = "is_false")]
    pub without_provider_volumes: bool,
}

impl PodSettings {
    /// Return `true` when no setting is set.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::PodSettings;
    ///
    /// let mut settings = PodSettings::default();
    /// assert!(settings.is_empty());
    /// settings.service_account = Some("agent".to_string());
    /// assert!(!settings.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.secret_env.is_empty()
            && self.service_account.is_none()
            && self.read_only_volumes.is_empty()
            && self.managed_settings.is_none()
            && self.pvc_volumes.is_empty()
            && !self.without_provider_volumes
    }
}

/// Add `entry` to `list`, replacing in place an entry with the same name.
pub(crate) fn upsert_secret_env(list: &mut Vec<SecretEnvVar>, entry: SecretEnvVar) {
    match list.iter_mut().find(|e| e.name == entry.name) {
        Some(existing) => *existing = entry,
        None => list.push(entry),
    }
}

/// 32-bit FNV-1a hash: deterministic across processes and platforms.
fn fnv1a(data: &[u8]) -> u32 {
    let mut hash: u32 = 0x811c_9dc5;
    for byte in data {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(0x0100_0193);
    }
    hash
}

fn is_label_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-')
}

fn is_valid_label_value(raw: &str) -> bool {
    !raw.is_empty()
        && raw.len() <= LABEL_VALUE_MAX
        && raw.chars().all(is_label_char)
        && raw.starts_with(|c: char| c.is_ascii_alphanumeric())
        && raw.ends_with(|c: char| c.is_ascii_alphanumeric())
}

/// Turn any string into a valid Kubernetes label value.
///
/// A value that is already valid is returned unchanged. Otherwise every char
/// outside `[A-Za-z0-9._-]` becomes `-`, the result is truncated, leading and
/// trailing non-alphanumerics are trimmed, an empty result becomes `unnamed`,
/// and `-` plus 8 hex chars of a stable hash of the raw input is appended, so
/// that `"a b"` and `"a/b"` do not collide. The output is at most 63 bytes and
/// deterministic.
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::sanitize_label_value;
///
/// assert_eq!(sanitize_label_value("investigate"), "investigate");
/// assert!(sanitize_label_value("fix bug/42").starts_with("fix-bug-42-"));
/// assert_ne!(sanitize_label_value("a b"), sanitize_label_value("a/b"));
/// ```
pub fn sanitize_label_value(raw: &str) -> String {
    if is_valid_label_value(raw) {
        return raw.to_string();
    }

    let replaced: String = raw
        .chars()
        .map(|c| if is_label_char(c) { c } else { '-' })
        .collect();
    // Only ASCII remains, so byte truncation cannot split a char.
    let truncated = &replaced[..replaced.len().min(LABEL_VALUE_MAX - HASH_SUFFIX_LEN)];
    let trimmed = truncated.trim_matches(|c: char| !c.is_ascii_alphanumeric());
    let base = match trimmed {
        "" => "unnamed",
        valid => valid,
    };
    format!("{base}-{:08x}", fnv1a(raw.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn k8s_sanitize_label_value_keeps_valid_value() {
        assert_eq!(sanitize_label_value("investigate"), "investigate");
        assert_eq!(sanitize_label_value("step-1.a_b"), "step-1.a_b");
    }

    #[test]
    fn k8s_sanitize_label_value_replaces_invalid_chars_and_adds_hash() {
        let value = sanitize_label_value("fix bug/42");
        assert!(value.starts_with("fix-bug-42-"), "got {value}");
        assert_eq!(value.len(), "fix-bug-42".len() + HASH_SUFFIX_LEN);
        let suffix = &value["fix-bug-42-".len()..];
        assert!(suffix.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn k8s_sanitize_label_value_distinguishes_similar_inputs() {
        assert_ne!(sanitize_label_value("a b"), sanitize_label_value("a/b"));
    }

    #[test]
    fn k8s_sanitize_label_value_truncates_long_input() {
        let raw = "x".repeat(200);
        let value = sanitize_label_value(&raw);
        assert!(value.len() <= LABEL_VALUE_MAX, "len {}", value.len());
        assert!(value.ends_with(|c: char| c.is_ascii_alphanumeric()));
        assert!(value.starts_with(|c: char| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn k8s_sanitize_label_value_trims_non_alphanumeric_edges() {
        let value = sanitize_label_value("--step--");
        assert!(value.starts_with("step-"), "got {value}");
    }

    #[test]
    fn k8s_sanitize_label_value_empty_and_unicode_only() {
        assert!(sanitize_label_value("").starts_with("unnamed-"));
        assert!(sanitize_label_value("日本語").starts_with("unnamed-"));
        assert_ne!(sanitize_label_value(""), sanitize_label_value("日本語"));
    }

    #[test]
    fn k8s_sanitize_label_value_is_deterministic() {
        let raw = "Résumé / step #3";
        assert_eq!(sanitize_label_value(raw), sanitize_label_value(raw));
        assert!(is_valid_label_value(&sanitize_label_value(raw)));
    }

    #[test]
    fn k8s_pod_settings_is_empty() {
        assert!(PodSettings::default().is_empty());
        let with_secret = PodSettings {
            secret_env: vec![SecretEnvVar::default()],
            ..PodSettings::default()
        };
        assert!(!with_secret.is_empty());
        let with_preset = PodSettings {
            managed_settings: Some("locked".to_string()),
            ..PodSettings::default()
        };
        assert!(!with_preset.is_empty());
        let with_volume = PodSettings {
            read_only_volumes: vec![ReadOnlyVolume {
                source: PodVolumeSource::HostPath {
                    path: "/srv".to_string(),
                },
                mount_path: "/data".to_string(),
                sub_path: None,
            }],
            ..PodSettings::default()
        };
        assert!(!with_volume.is_empty());
        let with_pvc = PodSettings {
            pvc_volumes: vec![PvcVolume {
                claim_name: "repos".to_string(),
                mount_path: "/data".to_string(),
                sub_path: None,
                read_only: false,
            }],
            ..PodSettings::default()
        };
        assert!(!with_pvc.is_empty());
        let without_provider = PodSettings {
            without_provider_volumes: true,
            ..PodSettings::default()
        };
        assert!(!without_provider.is_empty());
    }

    #[test]
    fn k8s_validate_pvc_sub_path_accepts_nested_and_spaces() {
        assert!(validate_pvc_sub_path("a").is_ok());
        assert!(validate_pvc_sub_path("a/b c/d.e").is_ok());
        assert!(validate_pvc_sub_path("..a/.b").is_ok());
    }

    #[test]
    fn k8s_validate_pvc_sub_path_refuses_structural_errors() {
        for bad in ["", "/abs", "a//b", "a/", ".", "..", "a/../b", "./a"] {
            assert!(validate_pvc_sub_path(bad).is_err(), "{bad:?} accepted");
        }
    }

    #[test]
    fn k8s_pvc_volume_serde_skips_defaults() {
        let volume = PvcVolume {
            claim_name: "c".to_string(),
            mount_path: "/m".to_string(),
            sub_path: None,
            read_only: false,
        };
        let json = serde_json::to_value(&volume).unwrap();
        assert!(json.get("sub_path").is_none());
        let back: PvcVolume =
            serde_json::from_value(serde_json::json!({"claim_name":"c","mount_path":"/m"}))
                .unwrap();
        assert_eq!(back, volume);
    }

    #[test]
    fn k8s_pod_volume_source_serde_is_tagged() {
        let source = PodVolumeSource::PersistentVolumeClaim {
            claim_name: "repos".to_string(),
        };
        let json = serde_json::to_value(&source).unwrap();
        assert_eq!(json["kind"], "persistent_volume_claim");
        assert_eq!(json["claim_name"], "repos");
        let back: PodVolumeSource = serde_json::from_value(json).unwrap();
        assert_eq!(back, source);
    }
}
