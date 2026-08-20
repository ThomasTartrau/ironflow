//! Persistent Kubernetes transport for Claude Code CLI.
//!
//! [`K8sPersistentProvider`] reuses a long-running worker pod and executes
//! commands via the Kubernetes exec API. Lower latency but shared state
//! between invocations.
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
//! use ironflow_core::providers::claude::K8sPersistentProvider;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let provider = K8sPersistentProvider::new("ghcr.io/my-org/claude-runner:latest")
//!     .pod_name("claude-worker")
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
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Status;
use kube::api::{Api, AttachParams, DeleteParams, PostParams};
use kube::runtime::wait::{await_condition, conditions};
use tokio::time;

use tracing::{debug, warn};

use crate::error::AgentError;
use crate::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture, LogSink};
use crate::providers::claude::common as claude_common;
use crate::providers::claude::common::DEFAULT_TIMEOUT;

use super::common::{
    DEFAULT_INPUT_INIT_IMAGE, ImagePullPolicy, K8sClusterConfig, K8sResources, PodConfig,
    build_credentials_prefix, build_pod_spec, create_client,
};

/// Extract the exit code from a K8s exec status object.
///
/// The exec API returns a [`Status`] object with a `status` field set to
/// `"Success"` or `"Failure"`. On failure, `details.causes` may contain
/// a cause with `reason: "ExitCode"` and the actual numeric code as
/// `message`.
///
/// Falls back to a heuristic (empty stdout + non-empty stderr = 1) when no
/// status is available. The K8s exec API does not always return a status
/// (depends on the container runtime and K8s version), so a hard error
/// here would break older clusters.
fn extract_exit_code(status: &Option<Status>, stdout: &str, stderr: &str) -> i32 {
    if let Some(s) = status {
        if s.status.as_deref() == Some("Success") {
            return 0;
        }
        if let Some(details) = &s.details
            && let Some(causes) = &details.causes
        {
            for cause in causes {
                if cause.reason.as_deref() == Some("ExitCode")
                    && let Some(msg) = &cause.message
                    && let Ok(code) = msg.parse::<i32>()
                {
                    return code;
                }
            }
        }
        return 1;
    }
    if stdout.is_empty() && !stderr.is_empty() {
        return 1;
    }
    0
}

/// [`AgentProvider`] that reuses a persistent Kubernetes worker pod.
///
/// The provider maintains a long-running pod (with a sleep loop) and executes
/// commands inside it via the Kubernetes exec API. If the pod does not exist
/// or is in a terminal state, a new one is created automatically.
///
/// This avoids pod startup latency on each invocation at the cost of shared
/// state between calls.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::claude::K8sPersistentProvider;
///
/// let provider = K8sPersistentProvider::new("my-registry/claude:v1")
///     .pod_name("claude-worker")
///     .namespace("ci");
/// ```
#[derive(Clone)]
pub struct K8sPersistentProvider {
    image: String,
    pod_name: String,
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

impl K8sPersistentProvider {
    /// Create a new persistent K8s provider with the given container image.
    pub fn new(image: &str) -> Self {
        Self {
            image: image.to_string(),
            pod_name: "claude-code-worker".to_string(),
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

    /// Set the worker pod name (default: `"claude-code-worker"`).
    pub fn pod_name(mut self, name: &str) -> Self {
        self.pod_name = name.to_string();
        self
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
    /// use ironflow_core::providers::claude::K8sPersistentProvider;
    ///
    /// let provider = K8sPersistentProvider::new("registry.gitlab.com/org/image:v1")
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
    pub fn cluster_config(mut self, config: K8sClusterConfig) -> Self {
        self.cluster_config = config;
        self
    }

    /// Override the default timeout (default: 5 minutes).
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Add a single provider-level pod label applied to the worker pod.
    ///
    /// Can be called multiple times. These labels are applied when the
    /// persistent worker pod is created.
    pub fn pod_label(mut self, key: &str, value: &str) -> Self {
        self.pod_labels.insert(key.to_string(), value.to_string());
        self
    }

    /// Replace the entire provider-level pod labels map.
    pub fn pod_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.pod_labels = labels;
        self
    }

    /// Ensure the worker pod is running, creating it if necessary.
    async fn ensure_pod_running(&self, pods: &Api<Pod>) -> Result<(), AgentError> {
        let needs_create = match pods.get(&self.pod_name).await {
            Ok(pod) => {
                let phase = pod
                    .status
                    .as_ref()
                    .and_then(|s| s.phase.as_deref())
                    .unwrap_or("Unknown");
                matches!(phase, "Failed" | "Succeeded" | "Unknown")
            }
            Err(kube::Error::Api(e)) if e.code == 404 => true,
            Err(e) => {
                return Err(AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to get worker pod status: {e}"),
                });
            }
        };

        if needs_create {
            // Delete old pod and wait for it to be gone before recreating
            if pods
                .delete(&self.pod_name, &DeleteParams::default())
                .await
                .is_ok()
            {
                // Poll until the pod is actually gone (max 30s)
                for _ in 0..60 {
                    match pods.get(&self.pod_name).await {
                        Err(kube::Error::Api(e)) if e.code == 404 => break,
                        Err(_) => break,
                        Ok(_) => time::sleep(Duration::from_millis(500)).await,
                    }
                }
            }

            debug!(pod = %self.pod_name, "creating persistent worker pod");

            let pod_spec = build_pod_spec(&PodConfig {
                name: &self.pod_name,
                image: &self.image,
                command: vec![
                    "sh".to_string(),
                    "-c".to_string(),
                    "trap 'exit 0' TERM; while true; do sleep 3600 & wait; done".to_string(),
                ],
                namespace: &self.namespace,
                resources: &self.resources,
                service_account: self.service_account.as_deref(),
                restart_policy: "Always",
                image_pull_policy: &self.image_pull_policy,
                env_vars: &self.env_vars,
                image_pull_secrets: &self.image_pull_secrets,
                extra_labels: &self.pod_labels,
                volumes: &[],
                pvc_volumes: &[],
                inputs: &[],
                input_init_image: DEFAULT_INPUT_INIT_IMAGE,
                prompt_configmap: None,
                prompt_mount_path: "",
            })?;

            pods.create(&PostParams::default(), &pod_spec)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to create worker pod: {e}"),
                })?;
        }

        // Wait for Running state
        time::timeout(
            Duration::from_secs(120),
            await_condition(pods.clone(), &self.pod_name, conditions::is_pod_running()),
        )
        .await
        .map_err(|_| AgentError::Timeout {
            limit: Duration::from_secs(120),
        })?
        .map_err(|e| AgentError::ProcessFailed {
            exit_code: -1,
            stderr: format!("failed waiting for worker pod to be running: {e}"),
        })?;

        Ok(())
    }
}

impl K8sPersistentProvider {
    /// Shared implementation for `invoke` and `invoke_with_logs`.
    async fn invoke_inner(
        &self,
        config: &AgentConfig,
        log_sink: Option<Arc<dyn LogSink>>,
    ) -> Result<AgentOutput, AgentError> {
        let forced = claude_common::force_verbose_for_streaming(config, log_sink.is_some());
        let config = forced.as_ref().unwrap_or(config);

        claude_common::validate_prompt_size(config)?;
        let built = claude_common::build_command(config)?;

        let claude_cmd = claude_common::build_shell_command(&self.claude_path, &built.args);
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

        debug!(
            pod = %self.pod_name,
            namespace = %self.namespace,
            model = %config.model,
            "executing in persistent K8s pod"
        );

        let start = Instant::now();

        let client = create_client(&self.cluster_config).await?;

        let pods: Api<Pod> = Api::namespaced(client, &self.namespace);

        self.ensure_pod_running(&pods).await?;

        let mut attach_params = AttachParams::default().stderr(true);
        if built.stdin_prompt.is_some() {
            attach_params = attach_params.stdin(true);
        }

        let mut attached = time::timeout(
            Duration::from_secs(30),
            pods.exec(&self.pod_name, vec!["sh", "-c", &full_cmd], &attach_params),
        )
        .await
        .map_err(|_| AgentError::Timeout {
            limit: Duration::from_secs(30),
        })?
        .map_err(|e| AgentError::ProcessFailed {
            exit_code: -1,
            stderr: format!("failed to exec in worker pod: {e}"),
        })?;

        let status_fut = attached.take_status();

        if let (Some(prompt), Some(mut stdin)) = (&built.stdin_prompt, attached.stdin()) {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(prompt.as_bytes())
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to write prompt to K8s exec stdin: {e}"),
                })?;
            stdin
                .shutdown()
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to close K8s exec stdin: {e}"),
                })?;
        }

        let stdout_reader = attached.stdout();
        let stderr_reader = attached.stderr();

        let mut stdout_buf = Vec::new();
        let mut stderr_buf = Vec::new();
        let mut exec_status = None;

        let collect_result = time::timeout(self.timeout, async {
            let stdout_fut = async {
                if let Some(reader) = stdout_reader {
                    let mut stream = tokio_util::io::ReaderStream::new(reader);
                    while let Some(Ok(chunk)) = stream.next().await {
                        claude_common::stream_lines(&chunk, "stdout", log_sink.as_ref());
                        stdout_buf.extend_from_slice(&chunk);
                    }
                }
            };
            let stderr_fut = async {
                if let Some(reader) = stderr_reader {
                    let mut stream = tokio_util::io::ReaderStream::new(reader);
                    while let Some(Ok(chunk)) = stream.next().await {
                        claude_common::stream_lines(&chunk, "stderr", log_sink.as_ref());
                        stderr_buf.extend_from_slice(&chunk);
                    }
                }
            };
            let status_wait = async {
                if let Some(fut) = status_fut {
                    exec_status = fut.await;
                }
            };
            tokio::join!(stdout_fut, stderr_fut, status_wait);
        })
        .await;

        if collect_result.is_err() {
            warn!(timeout = ?self.timeout, "K8s exec timed out");
            return Err(AgentError::Timeout {
                limit: self.timeout,
            });
        }

        let duration_ms = start.elapsed().as_millis() as u64;

        let stdout = String::from_utf8_lossy(&stdout_buf).to_string();
        let stderr = String::from_utf8_lossy(&stderr_buf).to_string();

        let exit_code = extract_exit_code(&exec_status, &stdout, &stderr);

        if exit_code != 0 {
            return claude_common::handle_nonzero_exit(
                exit_code,
                &stdout,
                &stderr,
                config,
                duration_ms,
                "persistent k8s",
            );
        }

        debug!(
            stdout_len = stdout.len(),
            "persistent pod claude process completed"
        );

        claude_common::parse_output(&stdout, config, duration_ms)
    }
}

impl AgentProvider for K8sPersistentProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(self.invoke_inner(config, None))
    }

    fn invoke_with_logs<'a>(
        &'a self,
        config: &'a AgentConfig,
        log_sink: Arc<dyn LogSink>,
    ) -> InvokeFuture<'a> {
        Box::pin(self.invoke_inner(config, Some(log_sink)))
    }
}

#[cfg(test)]
mod tests {
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::{StatusCause, StatusDetails};

    use super::*;

    #[test]
    fn persistent_provider_defaults() {
        let provider = K8sPersistentProvider::new("my-image:v1");
        assert_eq!(provider.image, "my-image:v1");
        assert_eq!(provider.pod_name, "claude-code-worker");
        assert_eq!(provider.namespace, "default");
        assert_eq!(provider.claude_path, "claude");
        assert!(provider.working_dir.is_none());
        assert!(provider.service_account.is_none());
        assert_eq!(provider.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn persistent_provider_builder_chain() {
        let provider = K8sPersistentProvider::new("img:v2")
            .pod_name("my-worker")
            .namespace("prod")
            .claude_path("/opt/claude")
            .working_dir("/app")
            .service_account("worker-sa")
            .timeout(Duration::from_secs(900));

        assert_eq!(provider.pod_name, "my-worker");
        assert_eq!(provider.namespace, "prod");
        assert_eq!(provider.claude_path, "/opt/claude");
        assert_eq!(provider.working_dir, Some("/app".to_string()));
        assert_eq!(provider.service_account, Some("worker-sa".to_string()));
        assert_eq!(provider.timeout, Duration::from_secs(900));
    }

    #[test]
    fn persistent_provider_image_pull_secrets() {
        let provider = K8sPersistentProvider::new("registry.gitlab.com/org/img:v1")
            .image_pull_secret("gitlab-registry");
        assert_eq!(provider.image_pull_secrets.len(), 1);
        assert_eq!(provider.image_pull_secrets[0], "gitlab-registry");
    }

    #[test]
    fn persistent_provider_clone() {
        let provider = K8sPersistentProvider::new("img")
            .pod_name("worker")
            .timeout(Duration::from_secs(42));
        let cloned = provider.clone();
        assert_eq!(cloned.pod_name, "worker");
        assert_eq!(cloned.timeout, Duration::from_secs(42));
    }

    #[test]
    fn persistent_provider_pod_labels_default_empty() {
        let provider = K8sPersistentProvider::new("img:v1");
        assert!(provider.pod_labels.is_empty());
    }

    #[test]
    fn persistent_provider_pod_labels_builder() {
        let mut labels = BTreeMap::new();
        labels.insert("env".to_string(), "staging".to_string());
        labels.insert("team".to_string(), "platform".to_string());
        let provider = K8sPersistentProvider::new("img:v1").pod_labels(labels);
        assert_eq!(provider.pod_labels.len(), 2);
        assert_eq!(provider.pod_labels["env"], "staging");
        assert_eq!(provider.pod_labels["team"], "platform");
    }

    #[test]
    fn persistent_provider_pod_label_builder() {
        let provider = K8sPersistentProvider::new("img:v1")
            .pod_label("env", "prod")
            .pod_label("team", "infra");
        assert_eq!(provider.pod_labels.len(), 2);
        assert_eq!(provider.pod_labels["env"], "prod");
        assert_eq!(provider.pod_labels["team"], "infra");
    }

    #[test]
    fn extract_exit_code_success_status() {
        let status = Some(Status {
            status: Some("Success".to_string()),
            ..Default::default()
        });
        assert_eq!(extract_exit_code(&status, "", ""), 0);
    }

    #[test]
    fn extract_exit_code_failure_with_exit_code_cause() {
        let status = Some(Status {
            status: Some("Failure".to_string()),
            details: Some(StatusDetails {
                causes: Some(vec![StatusCause {
                    reason: Some("ExitCode".to_string()),
                    message: Some("42".to_string()),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        });
        assert_eq!(extract_exit_code(&status, "", "error"), 42);
    }

    #[test]
    fn extract_exit_code_failure_without_cause_returns_1() {
        let status = Some(Status {
            status: Some("Failure".to_string()),
            ..Default::default()
        });
        assert_eq!(extract_exit_code(&status, "out", "err"), 1);
    }

    #[test]
    fn extract_exit_code_no_status_fallback_heuristic() {
        assert_eq!(extract_exit_code(&None, "some output", ""), 0);
        assert_eq!(extract_exit_code(&None, "", "some error"), 1);
        assert_eq!(extract_exit_code(&None, "", ""), 0);
    }
}
