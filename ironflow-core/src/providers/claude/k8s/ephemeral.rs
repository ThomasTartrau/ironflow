//! Ephemeral Kubernetes transport for Claude Code CLI.
//!
//! [`K8sEphemeralProvider`] creates a new pod for each invocation, reads logs,
//! then deletes the pod. Simple and isolated but has startup overhead.
//!
//! # Requirements
//!
//! * A reachable Kubernetes cluster (via kubeconfig or in-cluster config).
//! * The container image must include the `claude` binary.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//! use ironflow_core::providers::claude::K8sEphemeralProvider;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let provider = K8sEphemeralProvider::new("ghcr.io/my-org/claude-runner:latest")
//!     .namespace("ci");
//!
//! let result = Agent::new()
//!     .prompt("What is 2 + 2?")
//!     .run(&provider)
//!     .await?;
//!
//! println!("{}", result.text());
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, LogParams, PostParams};
use kube::runtime::wait::await_condition;
use tokio::time;

use tracing::{debug, warn};

use crate::error::AgentError;
use crate::provider::{AgentConfig, AgentProvider, InvokeFuture};
use crate::providers::claude::common as claude_common;
use crate::providers::claude::common::DEFAULT_TIMEOUT;

use super::common::{
    ImagePullPolicy, K8sClusterConfig, K8sResources, PodConfig, build_credentials_prefix,
    build_pod_spec, create_client, generate_pod_name,
};

/// Returns `true` when the pod has terminated (Succeeded or Failed).
fn is_pod_completed() -> impl kube::runtime::wait::Condition<Pod> {
    |obj: Option<&Pod>| {
        obj.and_then(|pod| pod.status.as_ref())
            .and_then(|status| status.phase.as_deref())
            .is_some_and(|phase| phase == "Succeeded" || phase == "Failed")
    }
}

/// [`AgentProvider`] that creates an ephemeral Kubernetes pod for each invocation.
///
/// For each call to [`invoke`](AgentProvider::invoke):
/// 1. Creates a pod with the configured image and the `claude` CLI command.
/// 2. Waits for the pod to complete (Succeeded or Failed phase).
/// 3. Reads the pod logs to extract the JSON output.
/// 4. Deletes the pod.
///
/// This provides full isolation between invocations at the cost of pod
/// startup latency (~5-30s depending on image pull policy).
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::claude::K8sEphemeralProvider;
///
/// let provider = K8sEphemeralProvider::new("registry.gitlab.com/org/claude:v1")
///     .namespace("ci")
///     .service_account("claude-sa")
///     .image_pull_secret("gitlab-registry");
/// ```
#[derive(Clone)]
pub struct K8sEphemeralProvider {
    image: String,
    namespace: String,
    claude_path: String,
    working_dir: Option<String>,
    resources: K8sResources,
    service_account: Option<String>,
    image_pull_policy: ImagePullPolicy,
    env_vars: Vec<(String, String)>,
    image_pull_secrets: Vec<String>,
    oauth_credentials: Option<String>,
    cluster_config: K8sClusterConfig,
    timeout: Duration,
    pod_labels: BTreeMap<String, String>,
}

impl K8sEphemeralProvider {
    /// Create a new ephemeral K8s provider with the given container image.
    pub fn new(image: &str) -> Self {
        Self {
            image: image.to_string(),
            namespace: "default".to_string(),
            claude_path: "claude".to_string(),
            working_dir: None,
            resources: K8sResources::default(),
            service_account: None,
            image_pull_policy: ImagePullPolicy::default(),
            env_vars: Vec::new(),
            image_pull_secrets: Vec::new(),
            oauth_credentials: None,
            cluster_config: K8sClusterConfig::default(),
            timeout: DEFAULT_TIMEOUT,
            pod_labels: BTreeMap::new(),
        }
    }

    /// Set the Kubernetes namespace (default: `"default"`).
    pub fn namespace(mut self, ns: &str) -> Self {
        self.namespace = ns.to_string();
        self
    }

    /// Override the path to the `claude` binary inside the container.
    pub fn claude_path(mut self, path: &str) -> Self {
        self.claude_path = path.to_string();
        self
    }

    /// Set the working directory inside the container.
    pub fn working_dir(mut self, dir: &str) -> Self {
        self.working_dir = Some(dir.to_string());
        self
    }

    /// Set CPU and memory limits for the pod.
    pub fn resources(mut self, resources: K8sResources) -> Self {
        self.resources = resources;
        self
    }

    /// Set the Kubernetes service account for the pod.
    pub fn service_account(mut self, sa: &str) -> Self {
        self.service_account = Some(sa.to_string());
        self
    }

    /// Set the image pull policy (default: [`IfNotPresent`](ImagePullPolicy::IfNotPresent)).
    pub fn image_pull_policy(mut self, policy: ImagePullPolicy) -> Self {
        self.image_pull_policy = policy;
        self
    }

    /// Set Claude OAuth credentials JSON to inject into the pod.
    ///
    /// The JSON is written to `~/.claude/.credentials.json` inside the container
    /// before the `claude` CLI is invoked. Format:
    ///
    /// ```json
    /// {"claudeAiOauth":{"accessToken":"sk-ant-oat01-...","refreshToken":"sk-ant-ort01-...","expiresAt":...}}
    /// ```
    ///
    /// You can retrieve this JSON on macOS with:
    /// ```sh
    /// security find-generic-password -s "Claude Code-credentials" -w
    /// ```
    pub fn oauth_credentials(mut self, json: &str) -> Self {
        self.oauth_credentials = Some(json.to_string());
        self
    }

    /// Add an image pull secret for pulling from private registries.
    ///
    /// The secret must already exist in the target namespace.
    /// Can be called multiple times to add several secrets.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::new("registry.gitlab.com/org/image:v1")
    ///     .image_pull_secret("gitlab-registry");
    /// ```
    pub fn image_pull_secret(mut self, secret_name: &str) -> Self {
        self.image_pull_secrets.push(secret_name.to_string());
        self
    }

    /// Add an environment variable to the container.
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env_vars.push((key.to_string(), value.to_string()));
        self
    }

    /// Set the Kubernetes cluster connection configuration.
    ///
    /// By default, uses `~/.kube/config` or in-cluster config.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::{K8sEphemeralProvider, K8sClusterConfig};
    ///
    /// // From a kubeconfig file
    /// let provider = K8sEphemeralProvider::new("img:v1")
    ///     .cluster_config(K8sClusterConfig::KubeconfigFile("/path/to/kubeconfig".to_string()));
    ///
    /// // From an inline YAML string
    /// let provider = K8sEphemeralProvider::new("img:v1")
    ///     .cluster_config(K8sClusterConfig::KubeconfigInline("apiVersion: v1\n...".to_string()));
    /// ```
    pub fn cluster_config(mut self, config: K8sClusterConfig) -> Self {
        self.cluster_config = config;
        self
    }

    /// Override the default timeout (default: 5 minutes).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Add a single provider-level pod label applied to every pod created.
    ///
    /// Can be called multiple times. These labels serve as defaults and are
    /// overridden by per-invocation labels from [`AgentConfig::pod_labels`].
    pub fn pod_label(mut self, key: &str, value: &str) -> Self {
        self.pod_labels.insert(key.to_string(), value.to_string());
        self
    }

    /// Replace the entire provider-level pod labels map.
    pub fn pod_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.pod_labels = labels;
        self
    }
}

impl AgentProvider for K8sEphemeralProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            claude_common::validate_prompt_size(config)?;
            let args = claude_common::build_args(config)?;

            let claude_cmd = claude_common::build_shell_command(&self.claude_path, &args);
            let creds_prefix = build_credentials_prefix(self.oauth_credentials.as_deref());
            let full_cmd = match (&self.working_dir, &config.working_dir) {
                (_, Some(dir)) | (Some(dir), None) => {
                    format!(
                        "{creds_prefix}cd {} && {}",
                        claude_common::build_shell_command(dir, &[]),
                        claude_cmd
                    )
                }
                (None, None) => format!("{creds_prefix}{claude_cmd}"),
            };

            let pod_name = generate_pod_name("claude-code");

            debug!(
                pod_name = %pod_name,
                namespace = %self.namespace,
                image = %self.image,
                model = %config.model,
                "creating ephemeral K8s pod"
            );

            let start = Instant::now();

            let client = create_client(&self.cluster_config).await?;

            let pods: Api<Pod> = Api::namespaced(client, &self.namespace);

            // Merge pod labels: provider defaults, then invocation overrides
            let mut merged_labels = self.pod_labels.clone();
            merged_labels.extend(config.pod_labels.clone());

            // Create pod
            let pod_spec = build_pod_spec(&PodConfig {
                name: &pod_name,
                image: &self.image,
                command: vec!["sh".to_string(), "-c".to_string(), full_cmd],
                namespace: &self.namespace,
                resources: &self.resources,
                service_account: self.service_account.as_deref(),
                restart_policy: "Never",
                image_pull_policy: &self.image_pull_policy,
                env_vars: &self.env_vars,
                image_pull_secrets: &self.image_pull_secrets,
                extra_labels: &merged_labels,
            })?;

            pods.create(&PostParams::default(), &pod_spec)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to create K8s pod: {e}"),
                })?;

            // Wait for pod to complete
            let wait_result = time::timeout(
                self.timeout,
                await_condition(pods.clone(), &pod_name, is_pod_completed()),
            )
            .await;

            // Determine exit code from pod phase (must read BEFORE deleting)
            let timed_out = wait_result.is_err();
            let pod_phase = if timed_out {
                "TimedOut".to_string()
            } else {
                // await_condition returns the pod when the condition is met
                let condition_result =
                    wait_result.expect("timeout already handled").map_err(|e| {
                        AgentError::ProcessFailed {
                            exit_code: -1,
                            stderr: format!("failed waiting for pod completion: {e}"),
                        }
                    })?;
                condition_result
                    .and_then(|p| p.status)
                    .and_then(|s| s.phase)
                    .unwrap_or_else(|| "Unknown".to_string())
            };

            // Read logs before cleanup
            let logs = pods
                .logs(&pod_name, &LogParams::default())
                .await
                .unwrap_or_default();

            // Delete pod (best-effort, after reading phase and logs)
            let _ = pods.delete(&pod_name, &DeleteParams::default()).await;

            if timed_out {
                warn!(timeout = ?self.timeout, pod = %pod_name, "K8s pod timed out");
                return Err(AgentError::Timeout {
                    limit: self.timeout,
                });
            }

            let duration_ms = start.elapsed().as_millis() as u64;
            let exit_code = if pod_phase == "Succeeded" { 0 } else { 1 };

            if exit_code != 0 {
                return claude_common::handle_nonzero_exit(
                    exit_code,
                    &logs,
                    "",
                    config,
                    duration_ms,
                    "ephemeral k8s",
                );
            }

            debug!(stdout_len = logs.len(), "ephemeral claude pod completed");

            claude_common::parse_output(&logs, config, duration_ms)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ephemeral_provider_defaults() {
        let provider = K8sEphemeralProvider::new("my-image:v1");
        assert_eq!(provider.image, "my-image:v1");
        assert_eq!(provider.namespace, "default");
        assert_eq!(provider.claude_path, "claude");
        assert!(provider.working_dir.is_none());
        assert!(provider.service_account.is_none());
        assert_eq!(provider.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn ephemeral_provider_builder_chain() {
        let provider = K8sEphemeralProvider::new("img:v2")
            .namespace("ci")
            .claude_path("/usr/bin/claude")
            .working_dir("/workspace")
            .service_account("claude-sa")
            .resources(K8sResources {
                cpu_limit: Some("1".to_string()),
                memory_limit: Some("2Gi".to_string()),
            })
            .timeout(Duration::from_secs(600));

        assert_eq!(provider.namespace, "ci");
        assert_eq!(provider.claude_path, "/usr/bin/claude");
        assert_eq!(provider.working_dir, Some("/workspace".to_string()));
        assert_eq!(provider.service_account, Some("claude-sa".to_string()));
        assert_eq!(provider.resources.cpu_limit, Some("1".to_string()));
        assert_eq!(provider.resources.memory_limit, Some("2Gi".to_string()));
        assert_eq!(provider.timeout, Duration::from_secs(600));
    }

    #[test]
    fn ephemeral_provider_image_pull_secrets() {
        let provider = K8sEphemeralProvider::new("registry.gitlab.com/org/img:v1")
            .image_pull_secret("gitlab-registry")
            .image_pull_secret("dockerhub");
        assert_eq!(provider.image_pull_secrets.len(), 2);
        assert_eq!(provider.image_pull_secrets[0], "gitlab-registry");
        assert_eq!(provider.image_pull_secrets[1], "dockerhub");
    }

    #[test]
    fn ephemeral_provider_clone() {
        let provider = K8sEphemeralProvider::new("img")
            .namespace("ns")
            .timeout(Duration::from_secs(42));
        let cloned = provider.clone();
        assert_eq!(cloned.namespace, "ns");
        assert_eq!(cloned.timeout, Duration::from_secs(42));
    }

    #[test]
    fn ephemeral_provider_pod_labels_default_empty() {
        let provider = K8sEphemeralProvider::new("img:v1");
        assert!(provider.pod_labels.is_empty());
    }

    #[test]
    fn ephemeral_provider_pod_labels_builder() {
        let mut labels = BTreeMap::new();
        labels.insert("env".to_string(), "staging".to_string());
        labels.insert("team".to_string(), "platform".to_string());
        let provider = K8sEphemeralProvider::new("img:v1").pod_labels(labels);
        assert_eq!(provider.pod_labels.len(), 2);
        assert_eq!(provider.pod_labels["env"], "staging");
        assert_eq!(provider.pod_labels["team"], "platform");
    }

    #[test]
    fn ephemeral_provider_pod_label_builder() {
        let provider = K8sEphemeralProvider::new("img:v1")
            .pod_label("env", "prod")
            .pod_label("team", "infra");
        assert_eq!(provider.pod_labels.len(), 2);
        assert_eq!(provider.pod_labels["env"], "prod");
        assert_eq!(provider.pod_labels["team"], "infra");
    }
}
