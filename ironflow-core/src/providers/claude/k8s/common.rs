//! Shared utilities for Kubernetes transport providers.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::Client;
use kube::config::{KubeConfigOptions, Kubeconfig};
use serde_json::json;

use crate::error::AgentError;
use crate::provider::AgentInput;

/// Default image used by the input-fetch initContainer.
///
/// Tiny (~5 MiB), pinned tag for reproducibility. Override at the provider level
/// when corporate registries forbid Docker Hub or when a specific curl version
/// is required.
pub const DEFAULT_INPUT_INIT_IMAGE: &str = "curlimages/curl:8.10.1";

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

/// Build a Kubernetes pod spec for running claude.
pub fn build_pod_spec(config: &PodConfig<'_>) -> Result<Pod, AgentError> {
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
        "app.kubernetes.io/managed-by".to_string(),
        "ironflow".to_string(),
    );
    labels.insert(
        "app.kubernetes.io/component".to_string(),
        "claude-runner".to_string(),
    );

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
                "env": super::super::common::env_vars_to_remove()
                    .iter()
                    .map(|var| json!({"name": var, "value": ""}))
                    .chain(config.env_vars.iter().map(|(k, v)| json!({"name": k, "value": v})))
                    .collect::<Vec<_>>()
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

    serde_json::from_value(pod_json).map_err(|e| AgentError::ProcessFailed {
        exit_code: -1,
        stderr: format!("failed to build K8s Pod spec: {e}"),
    })
}

#[cfg(test)]
mod tests {
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &vols,
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &pvcs,
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &vols,
            pvc_volumes: &pvcs,
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &inputs,
            input_init_image: "curlimages/curl:8.10.1",
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &inputs,
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: Some("my-pod-prompt"),
            prompt_mount_path: "/mnt/ironflow-prompt",
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
            volumes: &[],
            pvc_volumes: &[],
            inputs: &[],
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: "",
        })
        .unwrap();

        let spec = pod.spec.unwrap();
        assert!(spec.volumes.is_none());
    }
}
