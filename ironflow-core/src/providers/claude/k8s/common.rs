//! Shared utilities for Kubernetes transport providers.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::Client;
use kube::config::{KubeConfigOptions, Kubeconfig};
use serde_json::json;

use crate::error::AgentError;

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
/// The credentials JSON is written using a heredoc to avoid escaping issues.
/// Returns an empty string if no credentials are provided.
pub fn build_credentials_prefix(oauth_json: Option<&str>) -> String {
    match oauth_json {
        Some(json) => {
            // Escape single quotes in the JSON for safe shell embedding
            let escaped = json.replace('\'', "'\\''");
            format!(
                "mkdir -p $HOME/.claude && printf '%s' '{escaped}' > $HOME/.claude/.credentials.json && "
            )
        }
        None => String::new(),
    }
}

/// Generate a unique pod name with a random suffix.
pub fn generate_pod_name(prefix: &str) -> String {
    use std::time::SystemTime;
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{prefix}-{ts:x}")
}

/// Image pull policy for the Kubernetes pod.
#[derive(Clone, Default)]
pub enum ImagePullPolicy {
    /// Always pull the image from the registry.
    Always,
    /// Only pull if the image is not already present locally.
    #[default]
    IfNotPresent,
    /// Never pull — requires the image to be pre-loaded on the node.
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

/// Build a Kubernetes pod spec for running claude.
pub fn build_pod_spec(
    name: &str,
    image: &str,
    command: Vec<String>,
    namespace: &str,
    resources: &K8sResources,
    service_account: Option<&str>,
    restart_policy: &str,
    image_pull_policy: &ImagePullPolicy,
    env_vars: &[(String, String)],
) -> Pod {
    let mut resource_limits: BTreeMap<String, Quantity> = BTreeMap::new();
    if let Some(ref cpu) = resources.cpu_limit {
        resource_limits.insert("cpu".to_string(), Quantity(cpu.clone()));
    }
    if let Some(ref mem) = resources.memory_limit {
        resource_limits.insert("memory".to_string(), Quantity(mem.clone()));
    }

    let limits = if resource_limits.is_empty() {
        None
    } else {
        Some(json!({ "limits": resource_limits }))
    };

    let mut pod_json = json!({
        "apiVersion": "v1",
        "kind": "Pod",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": {
                "app.kubernetes.io/managed-by": "ironflow",
                "app.kubernetes.io/component": "claude-runner"
            }
        },
        "spec": {
            "restartPolicy": restart_policy,
            "containers": [{
                "name": "claude-code",
                "image": image,
                "imagePullPolicy": image_pull_policy.as_str(),
                "command": command,
                "env": std::iter::once(json!({"name": "CLAUDECODE", "value": ""}))
                    .chain(env_vars.iter().map(|(k, v)| json!({"name": k, "value": v})))
                    .collect::<Vec<_>>()
            }]
        }
    });

    if let Some(sa) = service_account {
        pod_json["spec"]["serviceAccountName"] = json!(sa);
    }
    if let Some(res) = limits {
        pod_json["spec"]["containers"][0]["resources"] = res;
    }

    serde_json::from_value(pod_json).expect("valid Pod spec")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn generate_pod_name_has_prefix() {
        let name = generate_pod_name("claude-code");
        assert!(name.starts_with("claude-code-"));
        assert!(name.len() > "claude-code-".len());
    }

    #[test]
    fn generate_pod_name_unique() {
        let name1 = generate_pod_name("test");
        std::thread::sleep(Duration::from_millis(2));
        let name2 = generate_pod_name("test");
        assert_ne!(name1, name2);
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
}
