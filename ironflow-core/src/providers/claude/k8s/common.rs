//! Shared utilities for Kubernetes transport providers.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::Client;
use kube::config::{KubeConfigOptions, Kubeconfig};
use serde_json::{Value, json};

use crate::error::AgentError;
use crate::provider::{
    AgentInput, LABEL_COMPONENT, LABEL_MANAGED_BY, MANAGED_BY_IRONFLOW, PodVolumeSource,
    ReadOnlyVolume, SecretEnvVar,
};
use crate::providers::claude::common::env_vars_to_remove;
use crate::providers::claude::k8s::profile::{ClaudeProfile, profile_mount_path};
use crate::providers::claude::k8s::toleration::K8sToleration;

/// Default image used by the input-fetch initContainer.
///
/// Tiny (~5 MiB), pinned tag for reproducibility. Override at the provider level
/// when corporate registries forbid Docker Hub or when a specific curl version
/// is required.
pub const DEFAULT_INPUT_INIT_IMAGE: &str = "curlimages/curl:8.10.1";

/// Uid (and gid) the sandboxed agent runs as, matching the official
/// `ironflow-claude-runner` image.
pub const SANDBOX_UID: i64 = 10001;

/// Home directory of the sandboxed agent, backed by an `emptyDir`.
pub const SANDBOX_HOME: &str = "/home/claude";

/// Directory Claude Code reads `managed-settings.json` from on Linux.
pub const MANAGED_SETTINGS_DIR: &str = "/etc/claude-code";

/// Directory holding the mounts of the Claude profile ConfigMaps (see
/// [`profile_mount_path`]), copied into `~/.claude` before the agent starts.
pub const PROFILE_MOUNT_DIR: &str = "/etc/ironflow/claude-profile";

/// Default margin added to the provider timeout for the pod deadline and the
/// expiry annotation.
pub(crate) const DEFAULT_DEADLINE_MARGIN: Duration = Duration::from_secs(60);

/// Key of the managed settings file inside its ConfigMap.
const MANAGED_SETTINGS_KEY: &str = "managed-settings.json";

/// Hardening applied to an agent pod by the sandboxed ephemeral provider.
///
/// Every field is optional: [`PodHardening::default`] leaves the pod spec
/// exactly as it was before hardening existed.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::common::{PodHardening, SandboxSettings};
///
/// let sandbox = SandboxSettings::default();
/// let hardening = PodHardening {
///     sandbox: Some(&sandbox),
///     ..PodHardening::default()
/// };
/// assert!(hardening.secret_env.is_empty());
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct PodHardening<'a> {
    /// Non-root, read-only root filesystem, dropped capabilities. `None`
    /// keeps the image's defaults.
    pub sandbox: Option<&'a SandboxSettings>,
    /// Environment variables read from Kubernetes Secrets.
    pub secret_env: &'a [SecretEnvVar],
    /// Volumes mounted read-only into the agent container.
    pub read_only_volumes: &'a [ReadOnlyVolume],
    /// ConfigMap holding `managed-settings.json`, mounted at [`MANAGED_SETTINGS_DIR`].
    pub managed_settings_configmap: Option<&'a str>,
    /// ConfigMaps holding a Claude profile, the n-th mounted read-only at
    /// [`profile_mount_path`]`(n)`.
    pub claude_profiles: &'a [ClaudeProfile],
    /// Annotations written into the pod metadata.
    pub annotations: Option<&'a BTreeMap<String, String>>,
}

/// Security settings of a sandboxed agent pod.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::common::{SANDBOX_UID, SandboxSettings};
///
/// let settings = SandboxSettings::default();
/// assert_eq!(settings.run_as_user, SANDBOX_UID);
/// assert!(!settings.writable_root);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxSettings {
    /// Uid, gid and fsGroup of the pod (default [`SANDBOX_UID`]).
    pub run_as_user: i64,
    /// Keep the root filesystem writable (default `false`).
    pub writable_root: bool,
    /// Size limit of the `emptyDir` backing [`SANDBOX_HOME`] (default `1Gi`).
    pub home_size_limit: String,
    /// Size limit of the `emptyDir` backing `/tmp` (default `512Mi`).
    pub tmp_size_limit: String,
    /// Time added to the provider timeout for the pod deadline and the expiry
    /// annotation (default 60s).
    pub deadline_margin: Duration,
}

impl Default for SandboxSettings {
    fn default() -> Self {
        Self {
            run_as_user: SANDBOX_UID,
            writable_root: false,
            home_size_limit: "1Gi".to_string(),
            tmp_size_limit: "512Mi".to_string(),
            deadline_margin: DEFAULT_DEADLINE_MARGIN,
        }
    }
}

/// Kubernetes cluster connection configuration.
///
/// Determines how the [`kube::Client`] connects to the cluster.
#[derive(Clone, Default)]
pub enum K8sClusterConfig {
    /// Use the default kubeconfig (`~/.kube/config` or in-cluster).
    #[default]
    Default,
    /// Load kubeconfig from a specific file path.
    KubeconfigFile(String),
    /// Parse kubeconfig from an inline YAML string.
    KubeconfigInline(String),
}

/// Create a [`kube::Client`] from the given cluster configuration.
pub async fn create_client(config: &K8sClusterConfig) -> Result<Client, AgentError> {
    match config {
        K8sClusterConfig::Default => {
            Client::try_default()
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to create K8s client from default config: {e}"),
                })
        }
        K8sClusterConfig::KubeconfigFile(path) => {
            let kubeconfig =
                Kubeconfig::read_from(path).map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to read kubeconfig from '{path}': {e}"),
                })?;
            let config =
                kube::Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
                    .await
                    .map_err(|e| AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!("failed to build K8s config from kubeconfig file: {e}"),
                    })?;
            Client::try_from(config).map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to create K8s client from kubeconfig file: {e}"),
            })
        }
        K8sClusterConfig::KubeconfigInline(yaml) => {
            let kubeconfig =
                Kubeconfig::from_yaml(yaml).map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to parse inline kubeconfig: {e}"),
                })?;
            let config =
                kube::Config::from_custom_kubeconfig(kubeconfig, &KubeConfigOptions::default())
                    .await
                    .map_err(|e| AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!("failed to build K8s config from inline kubeconfig: {e}"),
                    })?;
            Client::try_from(config).map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to create K8s client from inline kubeconfig: {e}"),
            })
        }
    }
}

/// Resource limits for the Kubernetes pod.
#[derive(Clone, Default)]
pub struct K8sResources {
    /// CPU limit (e.g. `"500m"`, `"2"`).
    pub cpu_limit: Option<String>,
    /// Memory limit (e.g. `"512Mi"`, `"2Gi"`).
    pub memory_limit: Option<String>,
}

/// Build a shell prefix that writes OAuth credentials to `~/.claude/.credentials.json`
/// before executing the main command.
///
/// Returns an empty string if no credentials are provided.
pub fn build_credentials_prefix(oauth_json: Option<&str>) -> String {
    match oauth_json {
        Some(json) => {
            // Escape single quotes in the JSON for safe shell embedding.
            // The JSON is passed as a separate argument to printf, not in the
            // format string, so printf specifiers like %s in the JSON are safe.
            let escaped = json.replace('\'', "'\\''");
            format!(
                "mkdir -p $HOME/.claude && printf '%s' '{escaped}' > $HOME/.claude/.credentials.json && "
            )
        }
        None => String::new(),
    }
}

/// Build a shell prefix that writes OAuth credentials read from the
/// environment variable `var` to `~/.claude/.credentials.json`.
///
/// Only the variable name enters the pod spec; its value comes from a
/// Kubernetes Secret through `secretKeyRef`.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::claude::k8s::common::build_credentials_from_env_prefix;
///
/// let prefix = build_credentials_from_env_prefix("IRONFLOW_CLAUDE_CREDENTIALS");
/// assert!(prefix.contains("$IRONFLOW_CLAUDE_CREDENTIALS"));
/// ```
pub fn build_credentials_from_env_prefix(var: &str) -> String {
    format!(
        r#"mkdir -p "$HOME/.claude" && printf '%s' "${var}" > "$HOME/.claude/.credentials.json" && "#
    )
}

/// Generate a unique pod name with a timestamp and random suffix.
///
/// Combines a millisecond timestamp with a random component to guarantee
/// uniqueness even when called multiple times within the same millisecond
/// (e.g. parallel steps in a workflow).
pub fn generate_pod_name(prefix: &str) -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::SystemTime;

    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let random_suffix = RandomState::new().build_hasher().finish() & 0xFFFF_FFFF;
    format!("{prefix}-{ts:x}-{random_suffix:08x}")
}

/// Image pull policy for the Kubernetes pod.
#[derive(Clone, Default)]
pub enum ImagePullPolicy {
    /// Always pull the image from the registry.
    Always,
    /// Only pull if the image is not already present locally.
    #[default]
    IfNotPresent,
    /// Never pull, requires the image to be pre-loaded on the node.
    Never,
}

impl ImagePullPolicy {
    fn as_str(&self) -> &str {
        match self {
            Self::Always => "Always",
            Self::IfNotPresent => "IfNotPresent",
            Self::Never => "Never",
        }
    }
}

/// Configuration for building a Kubernetes pod spec.
pub struct PodConfig<'a> {
    /// Pod name.
    pub name: &'a str,
    /// Container image.
    pub image: &'a str,
    /// Command to run in the container.
    pub command: Vec<String>,
    /// Kubernetes namespace.
    pub namespace: &'a str,
    /// CPU and memory limits.
    pub resources: &'a K8sResources,
    /// Service account name.
    pub service_account: Option<&'a str>,
    /// Pod restart policy (`"Never"`, `"Always"`, etc.).
    pub restart_policy: &'a str,
    /// Image pull policy.
    pub image_pull_policy: &'a ImagePullPolicy,
    /// Environment variables for the container.
    pub env_vars: &'a [(String, String)],
    /// Image pull secrets for private registries.
    pub image_pull_secrets: &'a [String],
    /// Extra labels to apply to the pod metadata.
    ///
    /// Merged with hardcoded ironflow labels. Hardcoded labels always win
    /// in case of conflict.
    pub extra_labels: &'a BTreeMap<String, String>,
    /// Node selector constraining which nodes the pod may be scheduled on.
    ///
    /// Each pair is a node label `key: value` the target node must carry.
    /// When empty, no `spec.nodeSelector` is written and the scheduler is
    /// free to place the pod on any node.
    pub node_selector: &'a BTreeMap<String, String>,
    /// Tolerations letting the pod schedule onto tainted nodes.
    ///
    /// When empty, no `spec.tolerations` is written. Each entry lets the pod
    /// tolerate one taint (e.g. a dedicated worker node's `NoSchedule` taint).
    pub tolerations: &'a [K8sToleration],
    /// Host-path volumes to mount into the container.
    ///
    /// Each tuple is `(host_path, container_path)`. An empty slice means
    /// no volumes are mounted.
    pub volumes: &'a [(String, String)],
    /// PersistentVolumeClaim volumes to mount into the container.
    ///
    /// Each tuple is `(claim_name, mount_path)`. The PVC must already exist
    /// in the target namespace. Requires `ReadWriteMany` access mode when
    /// multiple pods mount the same claim concurrently.
    pub pvc_volumes: &'a [(String, String)],
    /// Declarative inputs that must be fetched before the main container runs.
    ///
    /// When non-empty, an `initContainer` running [`PodConfig::input_init_image`]
    /// downloads each input via `curl` into a shared `emptyDir` volume mounted
    /// at the parent directory of `mount_path` on both the init and the main
    /// containers.
    pub inputs: &'a [AgentInput],
    /// Image used by the input-fetch initContainer.
    ///
    /// Defaults to [`DEFAULT_INPUT_INIT_IMAGE`]. Ignored when `inputs` is empty.
    pub input_init_image: &'a str,
    /// Name of a ConfigMap containing the prompt data, mounted as a volume.
    ///
    /// When `Some`, a volume is added that mounts the ConfigMap at
    /// [`prompt_mount_path`](Self::prompt_mount_path).
    pub prompt_configmap: Option<&'a str>,
    /// Mount path for the prompt ConfigMap volume.
    pub prompt_mount_path: &'a str,
    /// Hardening applied on top of the base spec. [`PodHardening::default`]
    /// changes nothing.
    pub hardening: PodHardening<'a>,
}

/// Return the directory part of an absolute path (everything before the last `/`).
///
/// Returns an empty string when there is no directory part, i.e. the path is
/// not absolute or has no `/`. Callers must validate inputs upstream.
fn parent_dir(path: &str) -> &str {
    match path.rsplit_once('/') {
        Some((dir, _)) if !dir.is_empty() => dir,
        Some(_) => "/",
        None => "",
    }
}

/// Quote a string for safe inclusion in a `sh -c` argument.
///
/// Wraps the value in single quotes and escapes any embedded single quotes.
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// Validate that every [`AgentInput::mount_path`] is an absolute path with a
/// non-trivial parent directory.
///
/// Returns an error explaining the offending path. The K8s providers reject
/// pods that would otherwise mount an `emptyDir` at `/` or at an empty path.
fn validate_inputs(inputs: &[AgentInput]) -> Result<(), AgentError> {
    for input in inputs {
        if !input.mount_path.starts_with('/') {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!(
                    "agent input mount_path must be absolute, got '{}'",
                    input.mount_path
                ),
            });
        }
        let parent = parent_dir(&input.mount_path);
        if parent.is_empty() || parent == "/" {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!(
                    "agent input mount_path must live under a directory (not '/'), got '{}'",
                    input.mount_path
                ),
            });
        }
    }
    Ok(())
}

/// Build the input-fetch initContainer JSON and the emptyDir volume + mount
/// definitions consumed by both the init and main containers.
///
/// Inputs that share the same parent directory share a single `emptyDir`
/// volume, mounted on both containers at that directory.
fn build_input_artifacts(
    inputs: &[AgentInput],
    init_image: &str,
    image_pull_policy: &ImagePullPolicy,
) -> (
    Vec<serde_json::Value>,    // volumes
    Vec<serde_json::Value>,    // shared volume_mounts (init + main)
    Option<serde_json::Value>, // initContainer
) {
    if inputs.is_empty() {
        return (Vec::new(), Vec::new(), None);
    }

    let mut by_dir: BTreeMap<String, Vec<&AgentInput>> = BTreeMap::new();
    for input in inputs {
        by_dir
            .entry(parent_dir(&input.mount_path).to_string())
            .or_default()
            .push(input);
    }

    let mut volumes = Vec::with_capacity(by_dir.len());
    let mut volume_mounts = Vec::with_capacity(by_dir.len());
    for (idx, dir) in by_dir.keys().enumerate() {
        let name = format!("ironflow-input-{idx}");
        volumes.push(json!({ "name": name, "emptyDir": {} }));
        volume_mounts.push(json!({ "name": name, "mountPath": dir }));
    }

    let mut script = String::from("set -e\n");
    for input in inputs {
        script.push_str(&format!(
            "curl -sSfL --retry 3 --max-time 300 {url} -o {path}\n",
            url = shell_quote(&input.url),
            path = shell_quote(&input.mount_path),
        ));
    }

    let init_container = json!({
        "name": "ironflow-input-fetch",
        "image": init_image,
        "imagePullPolicy": image_pull_policy.as_str(),
        "command": ["sh", "-c", script],
        "volumeMounts": volume_mounts.clone(),
    });

    (volumes, volume_mounts, Some(init_container))
}

/// Build the container `env` list.
///
/// Order: blanking entries for the host's `CLAUDE*` vars, plain values, then
/// `secretKeyRef` entries, then the sandbox `HOME`/`TMPDIR`. A blanking entry
/// is dropped when the same name is set explicitly or from a Secret, so a
/// name never appears twice with conflicting values.
fn build_env(config: &PodConfig<'_>) -> Vec<Value> {
    let hardening = &config.hardening;
    let is_set = |name: &str| {
        config.env_vars.iter().any(|(k, _)| k == name)
            || hardening.secret_env.iter().any(|s| s.name == name)
    };

    let mut env: Vec<Value> = env_vars_to_remove()
        .into_iter()
        .filter(|var| !is_set(var.as_str()))
        .map(|var| json!({"name": var, "value": ""}))
        .collect();
    for (k, v) in config.env_vars {
        env.push(json!({"name": k, "value": v}));
    }
    for s in hardening.secret_env {
        env.push(json!({
            "name": s.name,
            "valueFrom": { "secretKeyRef": { "name": s.secret, "key": s.key } }
        }));
    }
    if hardening.sandbox.is_some() {
        if !is_set("HOME") {
            env.push(json!({"name": "HOME", "value": SANDBOX_HOME}));
        }
        if !is_set("TMPDIR") {
            env.push(json!({"name": "TMPDIR", "value": "/tmp"}));
        }
    }
    env
}

fn hardening_error(stderr: String) -> AgentError {
    AgentError::ProcessFailed {
        exit_code: -1,
        stderr,
    }
}

/// Validate the hardening inputs of a pod config.
///
/// Rejects read-only mounts at a relative path, at a directory the sandbox
/// owns (`/`, [`SANDBOX_HOME`], `/tmp`, [`MANAGED_SETTINGS_DIR`],
/// [`PROFILE_MOUNT_DIR`] and below) or at a path already mounted, and secret
/// env entries with an empty name, secret or key.
fn validate_hardening(config: &PodConfig<'_>) -> Result<(), AgentError> {
    let reserved = [
        SANDBOX_HOME,
        "/tmp",
        MANAGED_SETTINGS_DIR,
        PROFILE_MOUNT_DIR,
    ];
    let profile_prefix = format!("{PROFILE_MOUNT_DIR}/");
    let mut seen: BTreeSet<&str> = config
        .volumes
        .iter()
        .chain(config.pvc_volumes.iter())
        .map(|(_, mount)| mount.trim_end_matches('/'))
        .collect();

    for volume in config.hardening.read_only_volumes {
        let path = volume.mount_path.as_str();
        if !path.starts_with('/') {
            return Err(hardening_error(format!(
                "read-only volume mount_path must be absolute, got '{path}'"
            )));
        }
        let normalized = path.trim_end_matches('/');
        let under_profiles = normalized.starts_with(&profile_prefix);
        if normalized.is_empty() || reserved.contains(&normalized) || under_profiles {
            return Err(hardening_error(format!(
                "read-only volume cannot be mounted at reserved path '{path}'"
            )));
        }
        if !seen.insert(normalized) {
            let message = format!("duplicate volume mount path '{path}'");
            return Err(hardening_error(message));
        }
    }

    for secret in config.hardening.secret_env {
        if secret.name.is_empty() || secret.secret.is_empty() || secret.key.is_empty() {
            return Err(hardening_error(format!(
                "secret env entry needs a non-empty name, secret and key, got name='{}' secret='{}' key='{}'",
                secret.name, secret.secret, secret.key
            )));
        }
    }
    Ok(())
}

/// Container-level `securityContext` of a sandboxed pod.
fn sandbox_container_security_context(sandbox: &SandboxSettings) -> Value {
    json!({
        "allowPrivilegeEscalation": false,
        "readOnlyRootFilesystem": !sandbox.writable_root,
        "runAsNonRoot": true,
        "capabilities": { "drop": ["ALL"] },
        "seccompProfile": { "type": "RuntimeDefault" }
    })
}

/// Volume and mount JSON for a [`ReadOnlyVolume`] named `name`.
fn read_only_volume_json(name: &str, volume: &ReadOnlyVolume) -> (Value, Value) {
    let source = match &volume.source {
        PodVolumeSource::PersistentVolumeClaim { claim_name } => json!({
            "name": name,
            "persistentVolumeClaim": { "claimName": claim_name, "readOnly": true }
        }),
        PodVolumeSource::HostPath { path } => json!({
            "name": name,
            "hostPath": { "path": path, "type": "Directory" }
        }),
        PodVolumeSource::ConfigMap { name: cm } => json!({
            "name": name,
            "configMap": { "name": cm }
        }),
    };
    let mut mount = json!({
        "name": name,
        "mountPath": volume.mount_path,
        "readOnly": true
    });
    if let Some(sub_path) = &volume.sub_path {
        mount["subPath"] = json!(sub_path);
    }
    (source, mount)
}

/// Build a Kubernetes pod spec for running claude.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] when an input or a hardening setting
/// is invalid, or when the resulting JSON is not a valid pod.
pub fn build_pod_spec(config: &PodConfig<'_>) -> Result<Pod, AgentError> {
    validate_hardening(config)?;
    validate_inputs(config.inputs)?;

    let mut resource_limits: BTreeMap<String, Quantity> = BTreeMap::new();
    if let Some(ref cpu) = config.resources.cpu_limit {
        resource_limits.insert("cpu".to_string(), Quantity(cpu.clone()));
    }
    if let Some(ref mem) = config.resources.memory_limit {
        resource_limits.insert("memory".to_string(), Quantity(mem.clone()));
    }

    let limits = if resource_limits.is_empty() {
        None
    } else {
        Some(json!({ "limits": resource_limits }))
    };

    let mut labels: BTreeMap<String, String> = config.extra_labels.clone();
    labels.insert(
        LABEL_MANAGED_BY.to_string(),
        MANAGED_BY_IRONFLOW.to_string(),
    );
    labels.insert(LABEL_COMPONENT.to_string(), "claude-runner".to_string());

    let mut pod_json = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": config.name,
            "namespace": config.namespace,
            "labels": labels
        },
        "spec": {
            "restartPolicy": config.restart_policy,
            "containers": [{
                "name": "claude-code",
                "image": config.image,
                "imagePullPolicy": config.image_pull_policy.as_str(),
                "command": &config.command,
                "env": build_env(config)
            }]
        }
    });

    let (input_volumes, input_mounts, init_container) = build_input_artifacts(
        config.inputs,
        config.input_init_image,
        config.image_pull_policy,
    );

    let mut volumes_json = Vec::new();
    let mut main_mounts_json = Vec::new();

    if !config.volumes.is_empty() {
        for (i, (host_path, container_path)) in config.volumes.iter().enumerate() {
            let name = format!("vol-{i}");
            volumes_json.push(json!({
                "name": name,
                "hostPath": { "path": host_path, "type": "Directory" }
            }));
            main_mounts_json.push(json!({
                "name": name,
                "mountPath": container_path
            }));
        }
    }
    if !config.pvc_volumes.is_empty() {
        for (i, (claim_name, mount_path)) in config.pvc_volumes.iter().enumerate() {
            let name = format!("pvc-{i}");
            volumes_json.push(json!({
                "name": name,
                "persistentVolumeClaim": { "claimName": claim_name }
            }));
            main_mounts_json.push(json!({
                "name": name,
                "mountPath": mount_path
            }));
        }
    }
    volumes_json.extend(input_volumes);
    main_mounts_json.extend(input_mounts);

    if let Some(cm_name) = config.prompt_configmap {
        volumes_json.push(json!({
            "name": "ironflow-prompt",
            "configMap": { "name": cm_name }
        }));
        main_mounts_json.push(json!({
            "name": "ironflow-prompt",
            "mountPath": config.prompt_mount_path,
            "readOnly": true
        }));
    }

    let hardening = &config.hardening;
    for (i, volume) in hardening.read_only_volumes.iter().enumerate() {
        let (vol, mount) = read_only_volume_json(&format!("ro-{i}"), volume);
        volumes_json.push(vol);
        main_mounts_json.push(mount);
    }
    if let Some(cm_name) = hardening.managed_settings_configmap {
        volumes_json.push(json!({
            "name": "ironflow-managed-settings",
            "configMap": {
                "name": cm_name,
                "items": [{ "key": MANAGED_SETTINGS_KEY, "path": MANAGED_SETTINGS_KEY }]
            }
        }));
        main_mounts_json.push(json!({
            "name": "ironflow-managed-settings",
            "mountPath": MANAGED_SETTINGS_DIR,
            "readOnly": true
        }));
    }
    for (i, profile) in hardening.claude_profiles.iter().enumerate() {
        let name = format!("ironflow-claude-profile-{i}");
        volumes_json.push(json!({ "name": name, "configMap": { "name": profile.configmap } }));
        main_mounts_json.push(json!({
            "name": name,
            "mountPath": profile_mount_path(i),
            "readOnly": true
        }));
    }
    if let Some(sandbox) = hardening.sandbox {
        // With a read-only root filesystem, HOME and /tmp must be writable
        // volumes of their own.
        volumes_json.push(json!({
            "name": "ironflow-home",
            "emptyDir": { "sizeLimit": sandbox.home_size_limit }
        }));
        main_mounts_json.push(json!({ "name": "ironflow-home", "mountPath": SANDBOX_HOME }));
        volumes_json.push(json!({
            "name": "ironflow-tmp",
            "emptyDir": { "sizeLimit": sandbox.tmp_size_limit }
        }));
        main_mounts_json.push(json!({ "name": "ironflow-tmp", "mountPath": "/tmp" }));
    }

    if !volumes_json.is_empty() {
        pod_json["spec"]["volumes"] = json!(volumes_json);
    }
    if !main_mounts_json.is_empty() {
        pod_json["spec"]["containers"][0]["volumeMounts"] = json!(main_mounts_json);
    }
    if let Some(init) = init_container {
        pod_json["spec"]["initContainers"] = json!([init]);
    }

    if let Some(sa) = config.service_account {
        pod_json["spec"]["serviceAccountName"] = json!(sa);
    }
    if !config.image_pull_secrets.is_empty() {
        pod_json["spec"]["imagePullSecrets"] = json!(
            config
                .image_pull_secrets
                .iter()
                .map(|s| json!({"name": s}))
                .collect::<Vec<_>>()
        );
    }
    if let Some(res) = limits {
        pod_json["spec"]["containers"][0]["resources"] = res;
    }
    if !config.node_selector.is_empty() {
        pod_json["spec"]["nodeSelector"] = json!(config.node_selector);
    }
    if !config.tolerations.is_empty() {
        pod_json["spec"]["tolerations"] = json!(config.tolerations);
    }

    if let Some(sandbox) = hardening.sandbox {
        pod_json["spec"]["securityContext"] = json!({
            "runAsNonRoot": true,
            "runAsUser": sandbox.run_as_user,
            "runAsGroup": sandbox.run_as_user,
            "fsGroup": sandbox.run_as_user,
            "seccompProfile": { "type": "RuntimeDefault" }
        });
        let container_ctx = sandbox_container_security_context(sandbox);
        pod_json["spec"]["containers"][0]["securityContext"] = container_ctx.clone();
        if pod_json["spec"].get("initContainers").is_some() {
            pod_json["spec"]["initContainers"][0]["securityContext"] = container_ctx;
        }
        // An explicit service account means the caller wants its token.
        if config.service_account.is_none() {
            pod_json["spec"]["automountServiceAccountToken"] = json!(false);
        }
    }
    if let Some(annotations) = hardening.annotations.filter(|a| !a.is_empty()) {
        pod_json["metadata"]["annotations"] = json!(annotations);
    }

    serde_json::from_value(pod_json).map_err(|e| AgentError::ProcessFailed {
        exit_code: -1,
        stderr: format!("failed to build K8s Pod spec: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::to_value;

    use crate::provider::LABEL_EXPIRES_AT;

    use super::*;

    #[test]
    fn generate_pod_name_has_prefix() {
        let name = generate_pod_name("claude-code");
        assert!(name.starts_with("claude-code-"));
        assert!(name.len() > "claude-code-".len());
    }

    #[test]
    fn generate_pod_name_unique_same_millisecond() {
        let name1 = generate_pod_name("test");
        let name2 = generate_pod_name("test");
        assert_ne!(
            name1, name2,
            "two calls in the same millisecond must produce different names"
        );
    }

    #[test]
    fn build_credentials_prefix_none() {
        assert_eq!(build_credentials_prefix(None), "");
    }

    #[test]
    fn build_credentials_prefix_writes_file() {
        let json = r#"{"claudeAiOauth":{"accessToken":"tok"}}"#;
        let prefix = build_credentials_prefix(Some(json));
        assert!(prefix.contains("mkdir -p $HOME/.claude"));
        assert!(prefix.contains(".credentials.json"));
        assert!(prefix.contains(json));
        assert!(prefix.ends_with("&& "));
    }

    #[test]
    fn k8s_resources_default() {
        let r = K8sResources::default();
        assert!(r.cpu_limit.is_none());
        assert!(r.memory_limit.is_none());
    }

    #[test]
    fn build_pod_spec_without_image_pull_secrets() {
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        assert!(pod.spec.unwrap().image_pull_secrets.is_none());
    }

    #[test]
    fn build_pod_spec_with_image_pull_secrets() {
        let secrets = vec!["gitlab-registry".to_string(), "dockerhub".to_string()];
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "registry.gitlab.com/org/img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &secrets,
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let pull_secrets = pod.spec.unwrap().image_pull_secrets.unwrap();
        assert_eq!(pull_secrets.len(), 2);
        assert_eq!(pull_secrets[0].name, "gitlab-registry");
        assert_eq!(pull_secrets[1].name, "dockerhub");
    }

    #[test]
    fn build_pod_spec_without_extra_labels() {
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let labels = pod.metadata.labels.unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels["app.kubernetes.io/managed-by"], "ironflow");
        assert_eq!(labels["app.kubernetes.io/component"], "claude-runner");
    }

    #[test]
    fn build_pod_spec_with_extra_labels() {
        let mut extra = BTreeMap::new();
        extra.insert("network-profile".to_string(), "restricted".to_string());
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &extra,
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let labels = pod.metadata.labels.unwrap();
        assert_eq!(labels.len(), 3);
        assert_eq!(labels["network-profile"], "restricted");
        assert_eq!(labels["app.kubernetes.io/managed-by"], "ironflow");
        assert_eq!(labels["app.kubernetes.io/component"], "claude-runner");
    }

    #[test]
    fn build_pod_spec_extra_labels_cannot_override_hardcoded() {
        let mut extra = BTreeMap::new();
        extra.insert(
            "app.kubernetes.io/managed-by".to_string(),
            "attacker".to_string(),
        );
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &extra,
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let labels = pod.metadata.labels.unwrap();
        assert_eq!(
            labels["app.kubernetes.io/managed-by"], "ironflow",
            "hardcoded label must not be overridden by extra_labels"
        );
    }

    #[test]
    fn build_pod_spec_merges_labels_correctly() {
        let mut extra = BTreeMap::new();
        extra.insert("team".to_string(), "observability".to_string());
        extra.insert(
            "ironflow.io/network-profile".to_string(),
            "grafana-only".to_string(),
        );
        extra.insert("env".to_string(), "staging".to_string());
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &extra,
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let labels = pod.metadata.labels.unwrap();
        assert_eq!(labels.len(), 5);
        assert_eq!(labels["team"], "observability");
        assert_eq!(labels["ironflow.io/network-profile"], "grafana-only");
        assert_eq!(labels["env"], "staging");
        assert_eq!(labels["app.kubernetes.io/managed-by"], "ironflow");
        assert_eq!(labels["app.kubernetes.io/component"], "claude-runner");
    }

    #[test]
    fn build_pod_spec_with_node_selector() {
        let mut selector = BTreeMap::new();
        selector.insert("kubernetes.io/hostname".to_string(), "ryzen1".to_string());
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &selector,
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let ns = pod
            .spec
            .unwrap()
            .node_selector
            .expect("nodeSelector present");
        assert_eq!(ns["kubernetes.io/hostname"], "ryzen1");
    }

    #[test]
    fn build_pod_spec_without_node_selector_omits_key() {
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        assert!(
            pod.spec.unwrap().node_selector.is_none(),
            "no nodeSelector key when the map is empty"
        );
    }

    #[test]
    fn build_pod_spec_without_volumes() {
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let spec = pod.spec.unwrap();
        assert!(spec.volumes.is_none());
        let container = &spec.containers[0];
        assert!(container.volume_mounts.is_none());
    }

    #[test]
    fn build_pod_spec_with_volumes() {
        let vols = vec![
            ("/tmp/worktrees".to_string(), "/data/worktrees".to_string()),
            ("/tmp/repos".to_string(), "/data/repos".to_string()),
        ];
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &vols,
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let spec = pod.spec.unwrap();

        let volumes = spec.volumes.unwrap();
        assert_eq!(volumes.len(), 2);
        assert_eq!(volumes[0].name, "vol-0");
        let hp0 = volumes[0].host_path.as_ref().unwrap();
        assert_eq!(hp0.path, "/tmp/worktrees");
        assert_eq!(hp0.type_.as_deref(), Some("Directory"));
        assert_eq!(volumes[1].name, "vol-1");
        let hp1 = volumes[1].host_path.as_ref().unwrap();
        assert_eq!(hp1.path, "/tmp/repos");
        assert_eq!(hp1.type_.as_deref(), Some("Directory"));

        let mounts = spec.containers[0].volume_mounts.as_ref().unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].name, "vol-0");
        assert_eq!(mounts[0].mount_path, "/data/worktrees");
        assert_eq!(mounts[1].name, "vol-1");
        assert_eq!(mounts[1].mount_path, "/data/repos");
    }

    #[test]
    fn build_pod_spec_with_pvc_volumes() {
        let pvcs = vec![
            ("jarvis-repos".to_string(), "/data/repos".to_string()),
            (
                "jarvis-worktrees".to_string(),
                "/data/worktrees".to_string(),
            ),
        ];
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &pvcs,
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let spec = pod.spec.unwrap();

        let volumes = spec.volumes.unwrap();
        assert_eq!(volumes.len(), 2);
        assert_eq!(volumes[0].name, "pvc-0");
        let pvc0 = volumes[0].persistent_volume_claim.as_ref().unwrap();
        assert_eq!(pvc0.claim_name, "jarvis-repos");
        assert_eq!(volumes[1].name, "pvc-1");
        let pvc1 = volumes[1].persistent_volume_claim.as_ref().unwrap();
        assert_eq!(pvc1.claim_name, "jarvis-worktrees");

        let mounts = spec.containers[0].volume_mounts.as_ref().unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].name, "pvc-0");
        assert_eq!(mounts[0].mount_path, "/data/repos");
        assert_eq!(mounts[1].name, "pvc-1");
        assert_eq!(mounts[1].mount_path, "/data/worktrees");
    }

    #[test]
    fn build_pod_spec_with_hostpath_and_pvc_volumes() {
        let vols = vec![("/tmp/cache".to_string(), "/cache".to_string())];
        let pvcs = vec![("data-pvc".to_string(), "/data".to_string())];
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &vols,
            pvc_volumes: &pvcs,
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        let spec = pod.spec.unwrap();

        let volumes = spec.volumes.unwrap();
        assert_eq!(volumes.len(), 2);
        assert!(volumes[0].host_path.is_some());
        assert!(volumes[1].persistent_volume_claim.is_some());

        let mounts = spec.containers[0].volume_mounts.as_ref().unwrap();
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].mount_path, "/cache");
        assert_eq!(mounts[1].mount_path, "/data");
    }

    #[test]
    fn parent_dir_extracts_directory() {
        assert_eq!(parent_dir("/work/dossier.pdf"), "/work");
        assert_eq!(parent_dir("/work/sub/dir/file.pdf"), "/work/sub/dir");
        assert_eq!(parent_dir("/file.pdf"), "/");
        assert_eq!(parent_dir("nofile"), "");
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("simple"), "'simple'");
        assert_eq!(shell_quote("with space"), "'with space'");
        assert_eq!(shell_quote("don't"), "'don'\\''t'");
    }

    #[test]
    fn validate_inputs_rejects_relative_paths() {
        let inputs = vec![AgentInput::new("https://x.com/f.pdf", "relative/path.pdf")];
        let err = validate_inputs(&inputs).unwrap_err();
        assert!(err.to_string().contains("must be absolute"));
    }

    #[test]
    fn validate_inputs_rejects_root_mount() {
        let inputs = vec![AgentInput::new("https://x.com/f.pdf", "/file.pdf")];
        let err = validate_inputs(&inputs).unwrap_err();
        assert!(err.to_string().contains("must live under a directory"));
    }

    #[test]
    fn build_pod_spec_with_single_input_creates_init_container() {
        let inputs = vec![AgentInput::new(
            "https://r2.example.com/dossier.pdf",
            "/work/dossier.pdf",
        )];
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &inputs,
            input_init_image: "curlimages/curl:8.10.1",
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();

        let spec = pod.spec.unwrap();
        let init_containers = spec.init_containers.expect("initContainer present");
        assert_eq!(init_containers.len(), 1);
        let init = &init_containers[0];
        assert_eq!(init.name, "ironflow-input-fetch");
        assert_eq!(init.image.as_deref(), Some("curlimages/curl:8.10.1"));
        let init_cmd = init.command.as_ref().unwrap();
        assert_eq!(init_cmd[0], "sh");
        assert_eq!(init_cmd[1], "-c");
        assert!(init_cmd[2].contains("curl -sSfL"));
        assert!(init_cmd[2].contains("https://r2.example.com/dossier.pdf"));
        assert!(init_cmd[2].contains("/work/dossier.pdf"));

        let init_mounts = init.volume_mounts.as_ref().unwrap();
        assert_eq!(init_mounts.len(), 1);
        assert_eq!(init_mounts[0].name, "ironflow-input-0");
        assert_eq!(init_mounts[0].mount_path, "/work");

        let main_mounts = spec.containers[0].volume_mounts.as_ref().unwrap();
        assert_eq!(main_mounts.len(), 1);
        assert_eq!(main_mounts[0].name, "ironflow-input-0");
        assert_eq!(main_mounts[0].mount_path, "/work");

        let volumes = spec.volumes.unwrap();
        assert_eq!(volumes.len(), 1);
        assert_eq!(volumes[0].name, "ironflow-input-0");
        assert!(volumes[0].empty_dir.is_some());
    }

    #[test]
    fn build_pod_spec_groups_inputs_by_parent_dir() {
        let inputs = vec![
            AgentInput::new("https://x.com/a.pdf", "/work/a.pdf"),
            AgentInput::new("https://x.com/b.pdf", "/work/b.pdf"),
            AgentInput::new("https://x.com/c.csv", "/data/c.csv"),
        ];
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &inputs,
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();

        let spec = pod.spec.unwrap();
        let volumes = spec.volumes.unwrap();
        assert_eq!(volumes.len(), 2, "/work and /data → 2 volumes");

        let main_mounts = spec.containers[0].volume_mounts.as_ref().unwrap();
        let mount_paths: Vec<_> = main_mounts.iter().map(|m| m.mount_path.as_str()).collect();
        assert!(mount_paths.contains(&"/work"));
        assert!(mount_paths.contains(&"/data"));

        let init = &spec.init_containers.unwrap()[0];
        let script = &init.command.as_ref().unwrap()[2];
        assert!(script.contains("/work/a.pdf"));
        assert!(script.contains("/work/b.pdf"));
        assert!(script.contains("/data/c.csv"));
    }

    #[test]
    fn build_pod_spec_no_inputs_no_init_container() {
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();
        assert!(pod.spec.unwrap().init_containers.is_none());
    }

    #[test]
    fn build_pod_spec_with_prompt_configmap_mounts_volume() {
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: Some("my-pod-prompt"),
            prompt_mount_path: "/mnt/ironflow-prompt",
            hardening: PodHardening::default(),
        })
        .unwrap();

        let spec = pod.spec.unwrap();
        let volumes = spec.volumes.expect("volumes present");
        assert!(volumes.iter().any(|v| v.name == "ironflow-prompt"));

        let mounts = spec.containers[0]
            .volume_mounts
            .as_ref()
            .expect("volume mounts present");
        let prompt_mount = mounts
            .iter()
            .find(|m| m.name == "ironflow-prompt")
            .expect("prompt mount present");
        assert_eq!(prompt_mount.mount_path, "/mnt/ironflow-prompt");
        assert_eq!(prompt_mount.read_only, Some(true));
    }

    #[test]
    fn build_pod_spec_without_prompt_configmap_no_extra_volume() {
        let pod = build_pod_spec(&PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &K8sResources::default(),
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &ImagePullPolicy::default(),
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &BTreeMap::new(),
            node_selector: &BTreeMap::new(),
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening: PodHardening::default(),
        })
        .unwrap();

        let spec = pod.spec.unwrap();
        assert!(spec.volumes.is_none());
    }

    // ── Hardening (sandboxed provider) ──────────────────────────────

    static NO_RESOURCES: K8sResources = K8sResources {
        cpu_limit: None,
        memory_limit: None,
    };
    static PULL_POLICY: ImagePullPolicy = ImagePullPolicy::IfNotPresent;
    static EMPTY_MAP: BTreeMap<String, String> = BTreeMap::new();

    /// A minimal pod config carrying `hardening`, everything else empty.
    fn hardened_config(hardening: PodHardening<'_>) -> PodConfig<'_> {
        PodConfig {
            name: "test-pod",
            image: "img:v1",
            command: vec!["sh".to_string()],
            namespace: "default",
            resources: &NO_RESOURCES,
            service_account: None,
            restart_policy: "Never",
            image_pull_policy: &PULL_POLICY,
            env_vars: &[],
            image_pull_secrets: &[],
            extra_labels: &EMPTY_MAP,
            node_selector: &EMPTY_MAP,
            tolerations: &[],
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
            hardening,
        }
    }

    fn pod_json(config: &PodConfig<'_>) -> Value {
        to_value(build_pod_spec(config).unwrap()).unwrap()
    }

    fn sandboxed(sandbox: &SandboxSettings) -> PodHardening<'_> {
        PodHardening {
            sandbox: Some(sandbox),
            ..PodHardening::default()
        }
    }

    fn find_by_name<'v>(list: &'v Value, name: &str) -> Option<&'v Value> {
        list.as_array()?.iter().find(|v| v["name"] == name)
    }

    fn secret(name: &str, secret: &str, key: &str) -> SecretEnvVar {
        SecretEnvVar {
            name: name.to_string(),
            secret: secret.to_string(),
            key: key.to_string(),
        }
    }

    fn ro(source: PodVolumeSource, mount_path: &str) -> ReadOnlyVolume {
        ReadOnlyVolume {
            source,
            mount_path: mount_path.to_string(),
            sub_path: None,
        }
    }

    /// A read-only ConfigMap volume mounted at `mount_path`.
    fn cm_ro(mount_path: &str) -> ReadOnlyVolume {
        let source = PodVolumeSource::ConfigMap {
            name: "cm".to_string(),
        };
        ro(source, mount_path)
    }

    #[test]
    fn build_pod_spec_sandbox_security_context() {
        let sandbox = SandboxSettings::default();
        let pod = pod_json(&hardened_config(sandboxed(&sandbox)));

        let pod_ctx = &pod["spec"]["securityContext"];
        assert_eq!(pod_ctx["runAsNonRoot"], true);
        assert_eq!(pod_ctx["runAsUser"], SANDBOX_UID);
        assert_eq!(pod_ctx["runAsGroup"], SANDBOX_UID);
        assert_eq!(pod_ctx["fsGroup"], SANDBOX_UID);
        assert_eq!(pod_ctx["seccompProfile"]["type"], "RuntimeDefault");

        let ctx = &pod["spec"]["containers"][0]["securityContext"];
        assert_eq!(ctx["readOnlyRootFilesystem"], true);
        assert_eq!(ctx["allowPrivilegeEscalation"], false);
        assert_eq!(ctx["runAsNonRoot"], true);
        assert_eq!(ctx["capabilities"]["drop"], json!(["ALL"]));
        assert_eq!(ctx["seccompProfile"]["type"], "RuntimeDefault");
    }

    #[test]
    fn build_pod_spec_sandbox_custom_uid() {
        let sandbox = SandboxSettings {
            run_as_user: 4242,
            ..SandboxSettings::default()
        };
        let pod = pod_json(&hardened_config(sandboxed(&sandbox)));
        assert_eq!(pod["spec"]["securityContext"]["runAsUser"], 4242);
        assert_eq!(pod["spec"]["securityContext"]["fsGroup"], 4242);
    }

    #[test]
    fn build_pod_spec_sandbox_writable_root() {
        let sandbox = SandboxSettings {
            writable_root: true,
            ..SandboxSettings::default()
        };
        let pod = pod_json(&hardened_config(sandboxed(&sandbox)));
        assert_eq!(
            pod["spec"]["containers"][0]["securityContext"]["readOnlyRootFilesystem"],
            false
        );
    }

    #[test]
    fn build_pod_spec_sandbox_init_container_is_hardened() {
        let sandbox = SandboxSettings::default();
        let inputs = vec![AgentInput::new("https://x.com/a.pdf", "/work/a.pdf")];
        let config = PodConfig {
            inputs: &inputs,
            ..hardened_config(sandboxed(&sandbox))
        };
        let pod = pod_json(&config);
        let init_ctx = &pod["spec"]["initContainers"][0]["securityContext"];
        assert_eq!(init_ctx["allowPrivilegeEscalation"], false);
        assert_eq!(init_ctx["capabilities"]["drop"], json!(["ALL"]));
    }

    #[test]
    fn build_pod_spec_sandbox_home_tmp_emptydir_size_limits() {
        let sandbox = SandboxSettings {
            home_size_limit: "2Gi".to_string(),
            tmp_size_limit: "256Mi".to_string(),
            ..SandboxSettings::default()
        };
        let pod = pod_json(&hardened_config(sandboxed(&sandbox)));

        let volumes = &pod["spec"]["volumes"];
        let home = find_by_name(volumes, "ironflow-home").expect("home volume");
        assert_eq!(home["emptyDir"]["sizeLimit"], "2Gi");
        let tmp = find_by_name(volumes, "ironflow-tmp").expect("tmp volume");
        assert_eq!(tmp["emptyDir"]["sizeLimit"], "256Mi");

        let mounts = &pod["spec"]["containers"][0]["volumeMounts"];
        let home_mount = find_by_name(mounts, "ironflow-home").unwrap();
        assert_eq!(home_mount["mountPath"], SANDBOX_HOME);
        let tmp_mount = find_by_name(mounts, "ironflow-tmp").unwrap();
        assert_eq!(tmp_mount["mountPath"], "/tmp");

        let env = &pod["spec"]["containers"][0]["env"];
        assert_eq!(find_by_name(env, "HOME").unwrap()["value"], SANDBOX_HOME);
        assert_eq!(find_by_name(env, "TMPDIR").unwrap()["value"], "/tmp");
    }

    #[test]
    fn build_pod_spec_sandbox_keeps_user_home() {
        let sandbox = SandboxSettings::default();
        let env_vars = vec![("HOME".to_string(), "/work".to_string())];
        let config = PodConfig {
            env_vars: &env_vars,
            ..hardened_config(sandboxed(&sandbox))
        };
        let pod = pod_json(&config);
        let env = pod["spec"]["containers"][0]["env"].as_array().unwrap();
        let homes: Vec<_> = env.iter().filter(|e| e["name"] == "HOME").collect();
        assert_eq!(homes.len(), 1);
        assert_eq!(homes[0]["value"], "/work");
    }

    #[test]
    fn build_pod_spec_sandbox_no_sa_disables_automount() {
        let sandbox = SandboxSettings::default();
        let pod = pod_json(&hardened_config(sandboxed(&sandbox)));
        assert_eq!(pod["spec"]["automountServiceAccountToken"], false);
    }

    #[test]
    fn build_pod_spec_sandbox_with_sa_omits_automount() {
        let sandbox = SandboxSettings::default();
        let config = PodConfig {
            service_account: Some("reader"),
            ..hardened_config(sandboxed(&sandbox))
        };
        let pod = pod_json(&config);
        assert!(pod["spec"].get("automountServiceAccountToken").is_none());
        assert_eq!(pod["spec"]["serviceAccountName"], "reader");
    }

    #[test]
    fn build_pod_spec_without_sandbox_has_no_security_context() {
        let pod = pod_json(&hardened_config(PodHardening::default()));
        let spec = &pod["spec"];
        assert!(spec.get("securityContext").is_none());
        assert!(spec["containers"][0].get("securityContext").is_none());
        assert!(spec.get("automountServiceAccountToken").is_none());
        assert!(spec.get("volumes").is_none());
        assert!(pod["metadata"].get("annotations").is_none());
        let env = &spec["containers"][0]["env"];
        assert!(find_by_name(env, "HOME").is_none());
        assert!(find_by_name(env, "TMPDIR").is_none());
    }

    #[test]
    fn build_pod_spec_secret_env_uses_secret_key_ref() {
        // IRONFLOW_ALLOW_BYPASS is always in env_vars_to_remove: a secret with
        // that name proves the blanking entry is filtered out.
        let secrets = vec![
            secret("CLAUDE_CODE_OAUTH_TOKEN", "claude-oauth", "token"),
            secret("IRONFLOW_ALLOW_BYPASS", "bypass", "flag"),
        ];
        let config = hardened_config(PodHardening {
            secret_env: &secrets,
            ..PodHardening::default()
        });
        let pod = pod_json(&config);
        let env_json = &pod["spec"]["containers"][0]["env"];
        let env = env_json.as_array().unwrap();

        let token = find_by_name(env_json, "CLAUDE_CODE_OAUTH_TOKEN").expect("token env");
        assert_eq!(token["valueFrom"]["secretKeyRef"]["name"], "claude-oauth");
        assert_eq!(token["valueFrom"]["secretKeyRef"]["key"], "token");
        assert!(token.get("value").is_none());

        let mut names: Vec<&str> = env.iter().map(|e| e["name"].as_str().unwrap()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "env names must be unique: {env:?}");

        let bypass = find_by_name(env_json, "IRONFLOW_ALLOW_BYPASS").unwrap();
        assert!(bypass.get("valueFrom").is_some());
    }

    #[test]
    fn build_pod_spec_explicit_env_drops_blanking_entry() {
        let env_vars = vec![("IRONFLOW_ALLOW_BYPASS".to_string(), "1".to_string())];
        let config = PodConfig {
            env_vars: &env_vars,
            ..hardened_config(PodHardening::default())
        };
        let pod = pod_json(&config);
        let env = pod["spec"]["containers"][0]["env"].as_array().unwrap();
        let entries: Vec<_> = env
            .iter()
            .filter(|e| e["name"] == "IRONFLOW_ALLOW_BYPASS")
            .collect();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["value"], "1");
    }

    #[test]
    fn build_pod_spec_read_only_volumes() {
        let volumes = vec![
            ro(
                PodVolumeSource::PersistentVolumeClaim {
                    claim_name: "repos".to_string(),
                },
                "/data/repos",
            ),
            ro(
                PodVolumeSource::HostPath {
                    path: "/srv/cache".to_string(),
                },
                "/data/cache",
            ),
            ReadOnlyVolume {
                source: PodVolumeSource::ConfigMap {
                    name: "guidelines".to_string(),
                },
                mount_path: "/data/guidelines/rules.md".to_string(),
                sub_path: Some("rules.md".to_string()),
            },
        ];
        let config = hardened_config(PodHardening {
            read_only_volumes: &volumes,
            ..PodHardening::default()
        });
        let pod = pod_json(&config);

        let vols = &pod["spec"]["volumes"];
        let pvc = find_by_name(vols, "ro-0").unwrap();
        assert_eq!(pvc["persistentVolumeClaim"]["claimName"], "repos");
        assert_eq!(pvc["persistentVolumeClaim"]["readOnly"], true);
        let host = find_by_name(vols, "ro-1").unwrap();
        assert_eq!(host["hostPath"]["path"], "/srv/cache");
        assert_eq!(host["hostPath"]["type"], "Directory");
        let cm = find_by_name(vols, "ro-2").unwrap();
        assert_eq!(cm["configMap"]["name"], "guidelines");

        let mounts = &pod["spec"]["containers"][0]["volumeMounts"];
        for (name, path) in [
            ("ro-0", "/data/repos"),
            ("ro-1", "/data/cache"),
            ("ro-2", "/data/guidelines/rules.md"),
        ] {
            let mount = find_by_name(mounts, name).unwrap();
            assert_eq!(mount["mountPath"], path);
            assert_eq!(mount["readOnly"], true);
        }
        let pvc_mount = find_by_name(mounts, "ro-0").unwrap();
        assert!(pvc_mount.get("subPath").is_none());
        let cm_mount = find_by_name(mounts, "ro-2").unwrap();
        assert_eq!(cm_mount["subPath"], "rules.md");
    }

    #[test]
    fn build_pod_spec_managed_settings_mount() {
        let config = hardened_config(PodHardening {
            managed_settings_configmap: Some("claude-managed-locked"),
            ..PodHardening::default()
        });
        let pod = pod_json(&config);
        let vol = find_by_name(&pod["spec"]["volumes"], "ironflow-managed-settings").unwrap();
        assert_eq!(vol["configMap"]["name"], "claude-managed-locked");
        assert_eq!(vol["configMap"]["items"][0]["key"], "managed-settings.json");
        assert_eq!(
            vol["configMap"]["items"][0]["path"],
            "managed-settings.json"
        );
        let mounts = &pod["spec"]["containers"][0]["volumeMounts"];
        let mount = find_by_name(mounts, "ironflow-managed-settings").unwrap();
        assert_eq!(mount["mountPath"], MANAGED_SETTINGS_DIR);
        assert_eq!(mount["readOnly"], true);
    }

    #[test]
    fn k8s_profile_mounts_one_read_only_sibling_per_configmap() {
        let profiles = [
            ClaudeProfile {
                configmap: "claude-profile".to_string(),
                subdir: String::new(),
            },
            ClaudeProfile {
                configmap: "claude-rules".to_string(),
                subdir: "rules".to_string(),
            },
        ];
        let pod = pod_json(&hardened_config(PodHardening {
            claude_profiles: &profiles,
            ..PodHardening::default()
        }));
        let mounts = &pod["spec"]["containers"][0]["volumeMounts"];
        for (i, configmap) in ["claude-profile", "claude-rules"].into_iter().enumerate() {
            let name = format!("ironflow-claude-profile-{i}");
            let vol = find_by_name(&pod["spec"]["volumes"], &name).unwrap();
            assert_eq!(vol["configMap"]["name"], configmap);
            let mount = find_by_name(mounts, &name).unwrap();
            assert_eq!(mount["mountPath"], profile_mount_path(i).as_str());
            assert_eq!(mount["readOnly"], true);
        }
    }

    #[test]
    fn build_pod_spec_annotations() {
        let mut annotations = BTreeMap::new();
        annotations.insert(LABEL_EXPIRES_AT.to_string(), "1700000000".to_string());
        let config = hardened_config(PodHardening {
            annotations: Some(&annotations),
            ..PodHardening::default()
        });
        let pod = pod_json(&config);
        assert_eq!(
            pod["metadata"]["annotations"][LABEL_EXPIRES_AT],
            "1700000000"
        );
    }

    fn hardening_err(config: &PodConfig<'_>) -> String {
        build_pod_spec(config).unwrap_err().to_string()
    }

    #[test]
    fn validate_hardening_rejects_relative_path() {
        let volumes = vec![cm_ro("data/x")];
        let config = hardened_config(PodHardening {
            read_only_volumes: &volumes,
            ..PodHardening::default()
        });
        assert!(hardening_err(&config).contains("must be absolute"));
    }

    #[test]
    fn validate_hardening_rejects_reserved_paths() {
        let profile_mount = profile_mount_path(0);
        let reserved = [
            "/",
            SANDBOX_HOME,
            "/tmp",
            "/tmp/",
            MANAGED_SETTINGS_DIR,
            PROFILE_MOUNT_DIR,
            &profile_mount,
        ];
        for path in reserved {
            let volumes = vec![cm_ro(path)];
            let config = hardened_config(PodHardening {
                read_only_volumes: &volumes,
                ..PodHardening::default()
            });
            assert!(
                hardening_err(&config).contains("reserved path"),
                "path {path} must be rejected"
            );
        }
    }

    #[test]
    fn validate_hardening_rejects_duplicate_mounts() {
        let volumes = vec![cm_ro("/data/x"), cm_ro("/data/x/")];
        let config = hardened_config(PodHardening {
            read_only_volumes: &volumes,
            ..PodHardening::default()
        });
        assert!(hardening_err(&config).contains("duplicate"));
    }

    #[test]
    fn validate_hardening_rejects_mount_clashing_with_pvc_volume() {
        let pvcs = vec![("repos".to_string(), "/data/repos".to_string())];
        let volumes = vec![cm_ro("/data/repos")];
        let config = PodConfig {
            pvc_volumes: &pvcs,
            ..hardened_config(PodHardening {
                read_only_volumes: &volumes,
                ..PodHardening::default()
            })
        };
        assert!(hardening_err(&config).contains("duplicate"));
    }

    #[test]
    fn validate_hardening_rejects_empty_secret_fields() {
        for entry in [
            secret("", "s", "k"),
            secret("VAR", "", "k"),
            secret("VAR", "s", ""),
        ] {
            let secrets = vec![entry];
            let config = hardened_config(PodHardening {
                secret_env: &secrets,
                ..PodHardening::default()
            });
            let err = hardening_err(&config);
            assert!(err.contains("non-empty name, secret and key"), "{err}");
        }
    }

    #[test]
    fn build_credentials_from_env_prefix_uses_var_only() {
        let prefix = build_credentials_from_env_prefix("IRONFLOW_CLAUDE_CREDENTIALS");
        assert!(prefix.contains("\"$IRONFLOW_CLAUDE_CREDENTIALS\""));
        assert!(prefix.contains(".credentials.json"));
        assert!(!prefix.contains('{'), "no JSON must be embedded: {prefix}");
        assert!(prefix.ends_with("&& "));
    }
}
