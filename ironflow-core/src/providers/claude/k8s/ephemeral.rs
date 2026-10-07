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
use std::env::var;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use futures_util::{AsyncBufReadExt, TryStreamExt};
use k8s_openapi::api::core::v1::{ConfigMap, PersistentVolumeClaim, Pod};
use kube::Client;
use kube::api::{Api, DeleteParams, LogParams, Patch, PatchParams, PostParams};
use kube::runtime::wait::await_condition;
use serde_json::{from_value, json};
use tokio::spawn;
use tokio::task::JoinHandle;
use tokio::time;

use tracing::{debug, info, warn};

use crate::account::ClaudeSubscriptionKind;
use crate::auth_proxy::{
    ADMIN_KEY_ENV, AuthProxyClient, AuthProxyError, IssuedToken, POD_BASE_URL_ENV, POD_TOKEN_ENV,
    ProxiedSecret, RELAY_PREFIX, SecretInjection, TokenRequest, resolve_credential,
};
use crate::error::AgentError;
use crate::provider::{
    AgentConfig, AgentOutput, AgentProvider, COMPONENT_ENVIRONMENT, EnvironmentVolume,
    InvokeFuture, LABEL_COMPONENT, LABEL_EGRESS_PROFILE, LABEL_EXPIRES_AT, LABEL_MANAGED_BY,
    LABEL_ROOT_RUN_ID, LABEL_RUN_ID, LABEL_STEP, LogSink, MANAGED_BY_IRONFLOW, PodVolumeSource,
    PvcVolume, ReadOnlyVolume, ReleaseFuture, SecretEnvVar, assert_pod_label_allowed,
    is_reserved_pod_label, upsert_proxied_secret, upsert_secret_env, validate_environment_id,
};
use crate::providers::claude::common as claude_common;
use crate::providers::claude::common::DEFAULT_TIMEOUT;
use crate::providers::claude::rate_limit_event;

use super::cleanup::{delete_and_wait, release_run, step_selection};
use super::common::{
    DEFAULT_DEADLINE_MARGIN, DEFAULT_INPUT_INIT_IMAGE, ImagePullPolicy, K8sClusterConfig,
    K8sResources, PodConfig, PodHardening, SandboxSettings, build_credentials_from_env_prefix,
    build_credentials_prefix, build_pod_spec, create_client, generate_pod_name,
};
use super::profile::{ClaudeProfile, build_profile_copy_prefix};
use super::reaper::{ReapReport, reap_orphans};
use super::toleration::K8sToleration;

/// Environment variable carrying the OAuth credentials JSON read from a Secret.
const CREDENTIALS_ENV_VAR: &str = "IRONFLOW_CLAUDE_CREDENTIALS";

/// Environment variables a sandboxed provider refuses as plain values.
const PLAIN_TEXT_SECRETS: [&str; 2] = ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"];

/// Environment variables the pod must not receive when the auth proxy is set.
const PROXY_FORBIDDEN_ENV: [&str; 5] = [
    "CLAUDE_CODE_OAUTH_TOKEN",
    "ANTHROPIC_API_KEY",
    POD_TOKEN_ENV,
    POD_BASE_URL_ENV,
    CREDENTIALS_ENV_VAR,
];

/// Turns off the non-essential traffic of Claude Code (telemetry, updates):
/// behind the auth proxy only the Messages API is reachable.
const NONESSENTIAL_TRAFFIC_ENV: &str = "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC";

/// Prefix of the variables git reads its config from
/// (`GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`).
const GIT_CONFIG_PREFIX: &str = "GIT_CONFIG_";

/// Number of git config entries passed through the environment.
const GIT_CONFIG_COUNT: &str = "GIT_CONFIG_COUNT";

/// The error raised when the pod would receive `name` while the auth proxy
/// is set. Names the variable, never its value.
fn proxy_forbidden(name: &str) -> AgentError {
    AgentError::ProcessFailed {
        exit_code: -1,
        stderr: format!(
            "auth_proxy is set: the pod must not receive {name}; the proxy injects the credential"
        ),
    }
}

/// An auth proxy failure, as the error of the invocation.
fn auth_proxy_error(e: AuthProxyError) -> AgentError {
    AgentError::ProcessFailed {
        exit_code: -1,
        stderr: format!("auth proxy: {e}"),
    }
}

/// The run and step labels of the proxy tokens of a pod: the pod name and
/// `agent` for an invocation outside a run.
fn grant_scope(run_id: Option<&String>, step: Option<&String>, pod: &str) -> (String, String) {
    let run_id = run_id.map_or_else(|| pod.to_string(), String::clone);
    let step = step.map_or_else(|| "agent".to_string(), String::clone);
    (run_id, step)
}

/// Delete the prompt ConfigMap of a pod that will not be created.
async fn abort_launch_configmap(configmaps: &Api<ConfigMap>, name: Option<&str>) {
    if let Some(name) = name
        && let Err(e) = configmaps.delete(name, &DeleteParams::default()).await
    {
        warn!(
            configmap = %name,
            error = %e,
            "failed to delete the prompt ConfigMap of an aborted pod"
        );
    }
}

/// Prefix of the name of a new environment claim.
const ENVIRONMENT_CLAIM_PREFIX: &str = "ironflow-env";

/// The PersistentVolumeClaim backing the environment of one agent pod.
#[derive(Debug, Clone, PartialEq, Eq)]
struct EnvironmentClaim {
    /// Claim name, handed out as the environment ID.
    name: String,
    /// `true` when this step creates the claim, `false` when it resumes one.
    created: bool,
}

/// Pick the claim of an agent pod: the one to resume, else a new name.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] when the ID to resume cannot name
/// a claim.
fn environment_claim_for(resume: Option<&str>) -> Result<EnvironmentClaim, AgentError> {
    match resume {
        Some(id) => {
            validate_environment_id(id).map_err(|reason| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: reason,
            })?;
            Ok(EnvironmentClaim {
                name: id.to_string(),
                created: false,
            })
        }
        None => Ok(EnvironmentClaim {
            name: generate_pod_name(ENVIRONMENT_CLAIM_PREFIX),
            created: true,
        }),
    }
}

/// The read-write mount of an environment claim, added to the step volumes.
fn environment_mount(volume: &EnvironmentVolume, claim: &str) -> PvcVolume {
    PvcVolume {
        claim_name: claim.to_string(),
        mount_path: volume.mount_path.clone(),
        sub_path: None,
        read_only: false,
    }
}

/// Labels of a new environment claim: the ironflow ownership labels, plus
/// the run and step labels of the pod when it has them.
fn environment_claim_labels(pod_labels: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    labels.insert(
        LABEL_MANAGED_BY.to_string(),
        MANAGED_BY_IRONFLOW.to_string(),
    );
    labels.insert(
        LABEL_COMPONENT.to_string(),
        COMPONENT_ENVIRONMENT.to_string(),
    );
    for key in [LABEL_ROOT_RUN_ID, LABEL_RUN_ID, LABEL_STEP] {
        if let Some(value) = pod_labels.get(key) {
            labels.insert(key.to_string(), value.clone());
        }
    }
    labels
}

/// Build a new `ReadWriteOnce` environment claim.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] when the claim cannot be built.
fn build_environment_claim(
    volume: &EnvironmentVolume,
    name: &str,
    namespace: &str,
    pod_labels: &BTreeMap<String, String>,
    expires_at: u64,
) -> Result<PersistentVolumeClaim, AgentError> {
    let mut spec = json!({
        "accessModes": ["ReadWriteOnce"],
        "resources": { "requests": { "storage": volume.size.to_quantity() } }
    });
    if let Some(class) = &volume.storage_class {
        spec["storageClassName"] = json!(class);
    }
    from_value(json!({
        "apiVersion": "v1",
        "kind": "PersistentVolumeClaim",
        "metadata": {
            "name": name,
            "namespace": namespace,
            "labels": environment_claim_labels(pod_labels),
            "annotations": { LABEL_EXPIRES_AT: expires_at.to_string() }
        },
        "spec": spec
    }))
    .map_err(|e| AgentError::ProcessFailed {
        exit_code: -1,
        stderr: format!("failed to build environment claim: {e}"),
    })
}

/// Return `true` when `pvc` is an ironflow environment claim that is not
/// being deleted: the only kind of claim a step may resume.
fn is_live_environment_claim(pvc: &PersistentVolumeClaim) -> bool {
    let labels = pvc.metadata.labels.as_ref();
    let label = |key: &str| labels.and_then(|l| l.get(key)).map(String::as_str);
    pvc.metadata.deletion_timestamp.is_none()
        && label(LABEL_MANAGED_BY) == Some(MANAGED_BY_IRONFLOW)
        && label(LABEL_COMPONENT) == Some(COMPONENT_ENVIRONMENT)
}

/// Hand the environment claim of the pod out with its output.
fn with_environment(mut output: AgentOutput, environment_id: Option<&str>) -> AgentOutput {
    output.environment_id = environment_id.map(str::to_string);
    output
}

/// Delete the environment claim of a pod that will not be created, when
/// this step created it. A resumed claim is never deleted.
async fn abort_environment_claim(claims: &Api<PersistentVolumeClaim>, claim: &EnvironmentClaim) {
    if !claim.created {
        return;
    }
    if let Err(e) = claims.delete(&claim.name, &DeleteParams::default()).await {
        warn!(
            environment = %claim.name,
            error = %e,
            "failed to delete the environment claim of an aborted pod"
        );
    }
}

/// Label selector matching every pod and Job created by ironflow: agent pods,
/// `PodRun` and `JobRun` of `ironflow-ops-k8s`.
pub(super) const MANAGED_SELECTOR: &str = "app.kubernetes.io/managed-by=ironflow";

/// Label selector matching every agent pod created by ironflow.
pub(super) const RUNNER_SELECTOR: &str =
    "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=claude-runner";

/// Label selector matching every prompt ConfigMap created by ironflow.
pub(super) const PROMPT_SELECTOR: &str =
    "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=prompt-data";

/// Label selector matching every environment claim created by ironflow.
pub(super) const ENVIRONMENT_SELECTOR: &str =
    "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=environment";

/// Current unix time in whole seconds.
pub(super) fn now_unix() -> Result<u64, AgentError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .map_err(|e| AgentError::ProcessFailed {
            exit_code: -1,
            stderr: format!("system clock is before the unix epoch: {e}"),
        })
}

fn is_terminal_phase(phase: &str) -> bool {
    phase == "Succeeded" || phase == "Failed"
}

/// Returns `true` when the pod has terminated (Succeeded or Failed).
fn is_pod_completed() -> impl kube::runtime::wait::Condition<Pod> {
    |obj: Option<&Pod>| {
        obj.and_then(|pod| pod.status.as_ref())
            .and_then(|status| status.phase.as_deref())
            .is_some_and(is_terminal_phase)
    }
}

/// Returns `true` when the pod is Running or already terminal (Succeeded/Failed).
fn is_pod_running_or_terminal() -> impl kube::runtime::wait::Condition<Pod> {
    |obj: Option<&Pod>| {
        obj.and_then(|pod| pod.status.as_ref())
            .and_then(|status| status.phase.as_deref())
            .is_some_and(|phase| phase == "Running" || is_terminal_phase(phase))
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
    volumes: Vec<(String, String)>,
    pvc_volumes: Vec<(String, String)>,
    input_init_image: String,
    node_selector: BTreeMap<String, String>,
    tolerations: Vec<K8sToleration>,
    active_deadline_seconds: Option<Duration>,
    sandbox: Option<SandboxSettings>,
    secret_env: Vec<SecretEnvVar>,
    oauth_credentials_secret: Option<(String, String)>,
    read_only_volumes: Vec<ReadOnlyVolume>,
    managed_settings_presets: BTreeMap<String, String>,
    default_managed_settings: Option<String>,
    /// Set by the builders of [`super::profile`].
    pub(super) claude_profiles: Vec<ClaudeProfile>,
    egress_profile: Option<String>,
    previous_attempt_timeout: Duration,
    runtime_class: Option<String>,
    auth_proxy_url: Option<String>,
    auth_proxy_admin_key: Option<String>,
    proxied_secrets: Vec<ProxiedSecret>,
    environment: Option<EnvironmentVolume>,
    sessions_claim: Option<String>,
}

/// Apply a Kubernetes `runtimeClassName` onto a built pod.
///
/// No-op unless both `runtime_class` and the pod spec are present, leaving the
/// field absent so the cluster default runtime applies.
fn apply_runtime_class(pod: &mut Pod, runtime_class: Option<&str>) {
    if let (Some(name), Some(spec)) = (runtime_class, pod.spec.as_mut()) {
        spec.runtime_class_name = Some(name.to_string());
    }
}

/// Apply a Kubernetes `activeDeadlineSeconds` onto a built pod, in whole seconds.
///
/// No-op when `deadline` is `None`, leaving the field absent. Sub-second
/// durations are truncated by [`Duration::as_secs`]; a value exceeding
/// [`i64::MAX`] seconds (unreachable for any real [`Duration`]) saturates to
/// [`i64::MAX`] rather than wrapping to a negative deadline.
fn apply_active_deadline_seconds(pod: &mut Pod, deadline: Option<Duration>) {
    if let (Some(d), Some(spec)) = (deadline, pod.spec.as_mut()) {
        spec.active_deadline_seconds = Some(i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
    }
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
            volumes: Vec::new(),
            pvc_volumes: Vec::new(),
            input_init_image: DEFAULT_INPUT_INIT_IMAGE.to_string(),
            node_selector: BTreeMap::new(),
            tolerations: Vec::new(),
            active_deadline_seconds: None,
            sandbox: None,
            secret_env: Vec::new(),
            oauth_credentials_secret: None,
            read_only_volumes: Vec::new(),
            managed_settings_presets: BTreeMap::new(),
            default_managed_settings: None,
            claude_profiles: Vec::new(),
            egress_profile: None,
            previous_attempt_timeout: Duration::from_secs(60),
            runtime_class: None,
            auth_proxy_url: None,
            auth_proxy_admin_key: None,
            proxied_secrets: Vec::new(),
            environment: None,
            sessions_claim: None,
        }
    }

    /// Create a hardened ephemeral provider for the given image.
    ///
    /// Same as [`new`](Self::new), plus a sandbox with the
    /// [`SandboxSettings`] defaults:
    ///
    /// * runs as uid/gid `10001`, `runAsNonRoot`, seccomp `RuntimeDefault`;
    /// * read-only root filesystem, all capabilities dropped, no privilege
    ///   escalation;
    /// * `HOME` (`/home/claude`, 1Gi) and `/tmp` (512Mi) on `emptyDir`s;
    /// * no service account token unless a service account is set;
    /// * `activeDeadlineSeconds` = timeout + 60s unless set explicitly;
    /// * refuses secrets as plain text ([`oauth_credentials`](Self::oauth_credentials),
    ///   `env("ANTHROPIC_API_KEY", ..)`, `env("CLAUDE_CODE_OAUTH_TOKEN", ..)`).
    ///
    /// Each default has an explicit relaxation method.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("registry.example.com/claude-runner:2.1.0-1")
    ///     .namespace("ironflow-agents")
    ///     .oauth_token_from_secret("claude-oauth", "token");
    /// ```
    pub fn sandboxed(image: &str) -> Self {
        Self {
            sandbox: Some(SandboxSettings::default()),
            ..Self::new(image)
        }
    }

    fn sandbox_mut(&mut self, method: &str) -> &mut SandboxSettings {
        self.sandbox.as_mut().unwrap_or_else(|| {
            panic!("{method} requires a provider built with K8sEphemeralProvider::sandboxed")
        })
    }

    /// Keep the root filesystem of a sandboxed pod writable.
    ///
    /// # Panics
    ///
    /// Panics if the provider was not built with [`sandboxed`](Self::sandboxed).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").allow_writable_root();
    /// ```
    pub fn allow_writable_root(mut self) -> Self {
        self.sandbox_mut("allow_writable_root").writable_root = true;
        self
    }

    /// Set the size limit of the `emptyDir` backing `HOME` (default `1Gi`).
    ///
    /// # Panics
    ///
    /// Panics if the provider was not built with [`sandboxed`](Self::sandboxed).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").home_size_limit("4Gi");
    /// ```
    pub fn home_size_limit(mut self, limit: &str) -> Self {
        self.sandbox_mut("home_size_limit").home_size_limit = limit.to_string();
        self
    }

    /// Set the size limit of the `emptyDir` backing `/tmp` (default `512Mi`).
    ///
    /// # Panics
    ///
    /// Panics if the provider was not built with [`sandboxed`](Self::sandboxed).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").tmp_size_limit("2Gi");
    /// ```
    pub fn tmp_size_limit(mut self, limit: &str) -> Self {
        self.sandbox_mut("tmp_size_limit").tmp_size_limit = limit.to_string();
        self
    }

    /// Set the margin added to the timeout for the pod deadline and the
    /// expiry annotation (default 60s).
    ///
    /// # Panics
    ///
    /// Panics if the provider was not built with [`sandboxed`](Self::sandboxed).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .deadline_margin(Duration::from_secs(120));
    /// ```
    pub fn deadline_margin(mut self, margin: Duration) -> Self {
        self.sandbox_mut("deadline_margin").deadline_margin = margin;
        self
    }

    /// Run the sandboxed pod as another uid (also used as gid and fsGroup).
    ///
    /// # Panics
    ///
    /// Panics if the provider was not built with [`sandboxed`](Self::sandboxed),
    /// or if `uid` is not strictly positive (root is never allowed).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").run_as_user(20000);
    /// ```
    pub fn run_as_user(mut self, uid: i64) -> Self {
        assert!(uid > 0, "run_as_user must be greater than 0");
        self.sandbox_mut("run_as_user").run_as_user = uid;
        self
    }

    /// Keep Claude Code sessions on a PersistentVolumeClaim so an agent step
    /// interrupted with its pod can resume its session in a new pod.
    ///
    /// `HOME` of a sandboxed pod is an `emptyDir`: without this volume the
    /// sessions die with the pod and an interrupted step restarts from
    /// scratch. The claim is mounted read-write at
    /// [`SESSIONS_MOUNT_PATH`](super::common::SESSIONS_MOUNT_PATH)
    /// (`~/.claude/projects`). Use a `ReadWriteMany` claim, or make sure the
    /// next pod lands on the same node. Claude Code keys sessions by working
    /// directory, so the steps must keep the same `cwd`. Claude profiles
    /// copied into `~/.claude/projects` are refused.
    ///
    /// # Panics
    ///
    /// Panics if the provider was not built with [`sandboxed`](Self::sandboxed),
    /// or if `claim_name` is empty.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").sessions_volume("claude-sessions");
    /// ```
    pub fn sessions_volume(mut self, claim_name: &str) -> Self {
        assert!(
            !claim_name.is_empty(),
            "sessions_volume claim name must not be empty"
        );
        self.sandbox_mut("sessions_volume");
        self.sessions_claim = Some(claim_name.to_string());
        self
    }

    /// Read an environment variable from a Kubernetes Secret.
    ///
    /// The pod gets `valueFrom.secretKeyRef`: the value never enters the pod
    /// spec. Calling it again with the same `var` replaces the entry. A step
    /// entry ([`AgentConfig::env_from_secret`]) with the same name wins.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .env_from_secret("ANTHROPIC_API_KEY", "anthropic", "api-key");
    /// ```
    pub fn env_from_secret(mut self, var: &str, secret: &str, key: &str) -> Self {
        let entry = SecretEnvVar {
            name: var.to_string(),
            secret: secret.to_string(),
            key: key.to_string(),
        };
        upsert_secret_env(&mut self.secret_env, entry);
        self
    }

    /// Read the Claude OAuth credentials JSON from a Kubernetes Secret.
    ///
    /// The JSON reaches the container through the
    /// `IRONFLOW_CLAUDE_CREDENTIALS` variable (`secretKeyRef`) and is written
    /// to `~/.claude/.credentials.json` before the agent starts. Only the
    /// variable name appears in the pod spec.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .oauth_credentials_from_secret("claude-credentials", "credentials.json");
    /// ```
    pub fn oauth_credentials_from_secret(mut self, secret: &str, key: &str) -> Self {
        self.oauth_credentials_secret = Some((secret.to_string(), key.to_string()));
        self
    }

    /// Read a long-lived Claude OAuth token (`claude setup-token`) from a
    /// Kubernetes Secret into `CLAUDE_CODE_OAUTH_TOKEN`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .oauth_token_from_secret("claude-oauth", "token");
    /// ```
    pub fn oauth_token_from_secret(self, secret: &str, key: &str) -> Self {
        self.env_from_secret("CLAUDE_CODE_OAUTH_TOKEN", secret, key)
    }

    /// Mount a volume read-only into every pod. Step volumes
    /// ([`AgentConfig::read_only_volume`]) come after these.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::provider::{PodVolumeSource, ReadOnlyVolume};
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").read_only_volume(ReadOnlyVolume {
    ///     source: PodVolumeSource::ConfigMap { name: "guidelines".to_string() },
    ///     mount_path: "/data/guidelines".to_string(),
    ///     sub_path: None,
    /// });
    /// ```
    pub fn read_only_volume(mut self, volume: ReadOnlyVolume) -> Self {
        self.read_only_volumes.push(volume);
        self
    }

    /// Mount a PersistentVolumeClaim read-only into every pod.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").read_only_pvc("repos", "/data/repos");
    /// ```
    pub fn read_only_pvc(self, claim: &str, mount_path: &str) -> Self {
        self.read_only_volume(ReadOnlyVolume {
            source: PodVolumeSource::PersistentVolumeClaim {
                claim_name: claim.to_string(),
            },
            mount_path: mount_path.to_string(),
            sub_path: None,
        })
    }

    /// Register a managed-settings preset: `name` is what steps select with
    /// [`AgentConfig::managed_settings`], `configmap` the ConfigMap holding
    /// `managed-settings.json`, mounted at `/etc/claude-code`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .managed_settings_preset("locked", "claude-managed-locked")
    ///     .managed_settings_preset("readonly", "claude-managed-readonly");
    /// ```
    pub fn managed_settings_preset(mut self, name: &str, configmap: &str) -> Self {
        self.managed_settings_presets
            .insert(name.to_string(), configmap.to_string());
        self
    }

    /// Select the managed-settings preset used by steps that do not pick one.
    ///
    /// The name must be registered with
    /// [`managed_settings_preset`](Self::managed_settings_preset), otherwise
    /// every invocation fails.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .managed_settings_preset("locked", "claude-managed-locked")
    ///     .default_managed_settings("locked");
    /// ```
    pub fn default_managed_settings(mut self, name: &str) -> Self {
        self.default_managed_settings = Some(name.to_string());
        self
    }

    /// Set the default network egress profile of every pod (the
    /// `ironflow.io/egress-profile` label). A step's
    /// [`AgentConfig::egress_profile`] wins.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").egress_profile("anthropic-only");
    /// ```
    pub fn egress_profile(mut self, name: &str) -> Self {
        self.egress_profile = Some(name.to_string());
        self
    }

    /// Set how long to wait for the pods of a previous attempt of the same
    /// step to terminate before starting a new one (default 60s).
    ///
    /// On timeout the invocation fails rather than running two agents side
    /// by side.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .previous_attempt_timeout(Duration::from_secs(120));
    /// ```
    pub fn previous_attempt_timeout(mut self, timeout: Duration) -> Self {
        self.previous_attempt_timeout = timeout;
        self
    }

    /// Route Claude traffic through an `ironflow-auth-proxy` at `url`: the pod
    /// never receives a Claude credential.
    ///
    /// At pod launch the worker asks the proxy for an opaque token bound to
    /// the run, the step and the pod expiry (`ironflow.io/expires-at`), and
    /// the pod receives `ANTHROPIC_BASE_URL=<url>`, `ANTHROPIC_AUTH_TOKEN=<opaque
    /// token>` and `CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC=1`. The proxy swaps
    /// the opaque token for the real credential. The token is revoked at the
    /// end of the step, and every token of a run when the run is released.
    ///
    /// The credential is the step's Provider Account, else the worker
    /// environment (`CLAUDE_CODE_OAUTH_TOKEN`, then `ANTHROPIC_API_KEY`). The
    /// admin key is read from `IRONFLOW_AUTH_PROXY_ADMIN_KEY` unless
    /// [`auth_proxy_admin_key`](Self::auth_proxy_admin_key) sets it. Any Claude
    /// credential set on the provider or the step for the pod
    /// ([`oauth_token_from_secret`](Self::oauth_token_from_secret),
    /// [`oauth_credentials`](Self::oauth_credentials), ...) fails the invocation.
    ///
    /// # Panics
    ///
    /// Panics if `url` does not start with `http://` or `https://`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .auth_proxy("http://ironflow-auth-proxy.ironflow-system");
    /// ```
    pub fn auth_proxy(mut self, url: &str) -> Self {
        assert!(
            url.starts_with("http://") || url.starts_with("https://"),
            "auth_proxy url must start with http:// or https://"
        );
        self.auth_proxy_url = Some(url.trim_end_matches('/').to_string());
        self
    }

    /// Set the admin key of the auth proxy instead of reading
    /// `IRONFLOW_AUTH_PROXY_ADMIN_KEY` from the worker environment.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .auth_proxy("http://ironflow-auth-proxy.ironflow-system")
    ///     .auth_proxy_admin_key("0123456789abcdef0123456789abcdef");
    /// ```
    pub fn auth_proxy_admin_key(mut self, key: &str) -> Self {
        self.auth_proxy_admin_key = Some(key.to_string());
        self
    }

    /// Hand a secret to every pod through the auth proxy. Requires
    /// [`auth_proxy`](Self::auth_proxy): an invocation without it fails.
    ///
    /// The pod gets `<env>` set to an opaque token and `<env>_URL` set to
    /// `<proxy>/r`, never the value. A request to `<env>_URL/<host>/<path>`
    /// carrying the token reaches `https://<host>/<path>` with the real
    /// secret, for the allowlisted hosts only. For a
    /// [`SecretInjection::Basic`] secret the pod also gets a git config
    /// (`GIT_CONFIG_COUNT`, `GIT_CONFIG_KEY_<n>`, `GIT_CONFIG_VALUE_<n>`) that
    /// rewrites `https://<host>/` to the proxy and sends the token as the
    /// Basic password, so `git clone https://<host>/..` works unchanged.
    /// Wildcard hosts get no git rewrite.
    ///
    /// Calling it again with the same `env` replaces the entry. A step entry
    /// ([`AgentConfig::proxied_secret`]) overrides a provider entry with the
    /// same `env`. The tokens are revoked with the step's Claude token.
    ///
    /// # Panics
    ///
    /// Panics when [`ProxiedSecret::validate`] refuses `secret`. The message
    /// never carries the value.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::env::{VarError, var};
    ///
    /// use ironflow_core::auth_proxy::{ProxiedSecret, SecretInjection};
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// # fn example() -> Result<(), VarError> {
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .auth_proxy("http://ironflow-auth-proxy.ironflow-system")
    ///     .proxied_secret(ProxiedSecret {
    ///         name: "GITLAB_TOKEN".to_string(),
    ///         env: "GITLAB_TOKEN".to_string(),
    ///         value: var("GITLAB_TOKEN")?,
    ///         injection: SecretInjection::Basic { username: "oauth2".to_string() },
    ///         hosts: vec!["gitlab.com".to_string()],
    ///     });
    /// # Ok(())
    /// # }
    /// ```
    pub fn proxied_secret(mut self, secret: ProxiedSecret) -> Self {
        upsert_proxied_secret(&mut self.proxied_secrets, secret);
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
    /// **The JSON lands in the pod spec in clear text**: anyone who can read
    /// pods in the namespace can read it. Prefer
    /// [`oauth_credentials_from_secret`](Self::oauth_credentials_from_secret).
    /// A [`sandboxed`](Self::sandboxed) provider rejects it at invocation time.
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

    /// Set a Kubernetes `activeDeadlineSeconds` on every pod this provider creates.
    ///
    /// Written into `PodSpec.activeDeadlineSeconds` as whole seconds (a
    /// sub-second `duration` is truncated). Unlike [`timeout`](Self::timeout),
    /// which bounds the caller's own wait loop client-side, this deadline is
    /// enforced by Kubernetes itself: the cluster kills the pod once it elapses,
    /// even if the calling process died (worker OOM, eviction, hard shutdown).
    /// It is an independent server-side safety net for orphaned pods.
    ///
    /// Opt-in: if this builder is never called the field is left absent and no
    /// pod deadline is set (unchanged behaviour).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::new("img:v1")
    ///     .active_deadline_seconds(Duration::from_secs(3600));
    /// ```
    pub fn active_deadline_seconds(mut self, duration: Duration) -> Self {
        self.active_deadline_seconds = Some(duration);
        self
    }

    /// Add a single provider-level pod label applied to every pod created.
    ///
    /// Can be called multiple times. These labels serve as defaults and are
    /// overridden by per-invocation labels from [`AgentConfig::pod_labels`].
    ///
    /// # Panics
    ///
    /// Panics on a label ironflow sets itself ([`is_reserved_pod_label`]).
    pub fn pod_label(mut self, key: &str, value: &str) -> Self {
        assert_pod_label_allowed(key);
        self.pod_labels.insert(key.to_string(), value.to_string());
        self
    }

    /// Replace the entire provider-level pod labels map.
    ///
    /// # Panics
    ///
    /// Panics under the same conditions as [`pod_label`](Self::pod_label).
    pub fn pod_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        labels.keys().for_each(|key| assert_pod_label_allowed(key));
        self.pod_labels = labels;
        self
    }

    /// Mount a host-path volume into the container.
    ///
    /// Can be called multiple times to add several volumes.
    ///
    /// A step drops this mount with [`AgentConfig::without_provider_volumes`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::new("img:v1")
    ///     .volume("/tmp/worktrees", "/data/worktrees")
    ///     .volume("/tmp/repos", "/data/repos");
    /// ```
    pub fn volume(mut self, host_path: &str, container_path: &str) -> Self {
        self.volumes
            .push((host_path.to_string(), container_path.to_string()));
        self
    }

    /// Mount a PersistentVolumeClaim into the container.
    ///
    /// The PVC must already exist in the target namespace. Use `ReadWriteMany`
    /// access mode when multiple pods mount the same claim concurrently.
    ///
    /// Can be called multiple times to add several PVC mounts.
    ///
    /// A step drops this mount with [`AgentConfig::without_provider_volumes`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::new("img:v1")
    ///     .pvc_volume("jarvis-repos", "/data/repos")
    ///     .pvc_volume("jarvis-worktrees", "/data/worktrees");
    /// ```
    pub fn pvc_volume(mut self, claim_name: &str, mount_path: &str) -> Self {
        self.pvc_volumes
            .push((claim_name.to_string(), mount_path.to_string()));
        self
    }

    /// Give every agent pod a persistent working volume.
    ///
    /// A step without [`AgentConfig::resume_environment`] gets a new
    /// `ReadWriteOnce` PersistentVolumeClaim named `ironflow-env-...`; a step
    /// with it mounts the claim of the previous step again, so it finds the
    /// files that step left. The claim name is returned in
    /// [`AgentOutput::environment_id`]. Point
    /// [`working_dir`](Self::working_dir) at the mount path to run the agent
    /// inside it.
    ///
    /// Claims outlive the pod: their expiry annotation is pushed
    /// [`ttl`](EnvironmentVolume::ttl) forward on every use, and
    /// [`reap_orphans`](Self::reap_orphans) deletes the expired ones. The
    /// worker needs `create`, `get`, `patch`, `list` and `delete` on
    /// `persistentvolumeclaims`.
    ///
    /// # Panics
    ///
    /// Panics if [`EnvironmentVolume::validate`] refuses `volume`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::provider::{EnvironmentVolume, StorageUnit, VolumeSize};
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1")
    ///     .environment_volume(
    ///         EnvironmentVolume::new("/workspace").size(VolumeSize::new(20, StorageUnit::Gi)),
    ///     )
    ///     .working_dir("/workspace");
    /// ```
    pub fn environment_volume(mut self, volume: EnvironmentVolume) -> Self {
        if let Err(reason) = volume.validate() {
            panic!("invalid environment volume: {reason}");
        }
        self.environment = Some(volume);
        self
    }

    /// Override the image used by the input-fetch initContainer.
    ///
    /// Defaults to [`DEFAULT_INPUT_INIT_IMAGE`] (`curlimages/curl:8.10.1`).
    /// The image only needs `sh` and `curl`. It is pulled with the same
    /// [`ImagePullPolicy`] as the main container.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::new("img:v1")
    ///     .input_init_image("registry.gitlab.com/org/curl:internal");
    /// ```
    pub fn input_init_image(mut self, image: &str) -> Self {
        self.input_init_image = image.to_string();
        self
    }

    /// Constrain the agent pod to nodes carrying a given label.
    ///
    /// Inserts a `key: value` pair into the pod's `spec.nodeSelector`. Call
    /// multiple times to require several labels; the scheduler only places the
    /// pod on nodes matching every pair. With no call, the pod may land on any
    /// schedulable node.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::new("img:v1")
    ///     .node_selector("kubernetes.io/hostname", "ryzen1");
    /// ```
    pub fn node_selector(mut self, key: &str, value: &str) -> Self {
        self.node_selector
            .insert(key.to_string(), value.to_string());
        self
    }

    /// Run the agent pods under a Kubernetes RuntimeClass.
    ///
    /// Sets `spec.runtimeClassName`. The typical use is `gvisor`, which runs
    /// the pod in a user-space kernel instead of sharing the node kernel the
    /// way `runc` does, for steps that execute untrusted code. A step's
    /// [`AgentConfig::runtime_class`] wins over this value. With no call the
    /// field stays absent and the cluster default runtime applies.
    ///
    /// # Panics
    ///
    /// Panics if `name` is empty or whitespace-only.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// let provider = K8sEphemeralProvider::new("img:v1").runtime_class("gvisor");
    /// ```
    pub fn runtime_class(mut self, name: &str) -> Self {
        assert!(!name.trim().is_empty(), "runtime class must not be empty");
        self.runtime_class = Some(name.to_string());
        self
    }

    /// Let the agent pod schedule onto tainted nodes.
    ///
    /// Appends one [`K8sToleration`] to the pod's `spec.tolerations`. Call
    /// multiple times to tolerate several taints. Without any call the pod
    /// carries no toleration and stays `Pending` on a `NoSchedule`-tainted node
    /// (e.g. a dedicated worker node).
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::{
    ///     K8sEphemeralProvider, K8sToleration, TolerationEffect, TolerationOperator,
    /// };
    ///
    /// let provider = K8sEphemeralProvider::new("img:v1").toleration(K8sToleration {
    ///     key: "dedicated".to_string(),
    ///     operator: TolerationOperator::Equal,
    ///     value: Some("worker".to_string()),
    ///     effect: TolerationEffect::NoSchedule,
    ///     toleration_seconds: None,
    /// });
    /// ```
    pub fn toleration(mut self, toleration: K8sToleration) -> Self {
        self.tolerations.push(toleration);
        self
    }
}

/// Path inside the container where the prompt ConfigMap is mounted.
const PROMPT_MOUNT_PATH: &str = "/mnt/ironflow-prompt";

/// Key used inside the ConfigMap for the prompt data.
const PROMPT_CM_KEY: &str = "prompt";

/// Shared context after pod creation: the pod API handle, pod name, and start time.
struct CreatedPod {
    pods: Api<Pod>,
    pod_name: String,
    start: Instant,
    prompt_configmap: Option<String>,
    configmaps: Option<Api<ConfigMap>>,
    /// Ids of the auth proxy tokens issued for the pod (Claude and proxied
    /// secrets), revoked at the end of the step.
    proxy_token_ids: Vec<String>,
    /// Name of the environment claim mounted in the pod, handed out as
    /// [`AgentOutput::environment_id`].
    environment_id: Option<String>,
}

/// Pod inputs merged from the provider defaults and the step's [`AgentConfig`].
#[derive(Debug)]
struct MergedPodInputs<'a> {
    volumes: &'a [(String, String)],
    pvc_volumes: &'a [(String, String)],
    secret_env: Vec<SecretEnvVar>,
    service_account: Option<String>,
    read_only_volumes: Vec<ReadOnlyVolume>,
    managed_settings_configmap: Option<String>,
    labels: BTreeMap<String, String>,
    runtime_class: Option<String>,
    proxied_secrets: Vec<ProxiedSecret>,
}

impl K8sEphemeralProvider {
    /// Merge the provider settings with the step's, the step winning.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when a sandboxed provider carries
    /// a secret as plain text, when the auth proxy is set and the pod would
    /// receive a Claude credential, when a proxied secret has no auth proxy or
    /// collides with another variable of the pod, when the managed-settings
    /// preset is unknown,
    /// or when the step resumes an environment and the provider has no
    /// [`environment_volume`](Self::environment_volume).
    fn merged_pod_inputs<'a>(
        &'a self,
        config: &AgentConfig,
    ) -> Result<MergedPodInputs<'a>, AgentError> {
        if self.environment.is_none() && config.resume_environment_id.is_some() {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: "resume_environment needs K8sEphemeralProvider::environment_volume"
                    .to_string(),
            });
        }
        if self.auth_proxy_url.is_some() {
            self.check_no_proxy_credential()?;
        }
        if self.sandbox.is_some() {
            if self.oauth_credentials.is_some() {
                return Err(AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: "sandboxed provider refuses inline oauth_credentials; use oauth_credentials_from_secret".to_string(),
                });
            }
            if let Some((key, _)) = self
                .env_vars
                .iter()
                .find(|(k, _)| PLAIN_TEXT_SECRETS.contains(&k.as_str()))
            {
                return Err(AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!(
                        "sandboxed provider refuses {key} as a plain env var; use env_from_secret"
                    ),
                });
            }
        }

        let mut secret_env = self.secret_env.clone();
        if let Some((secret, key)) = &self.oauth_credentials_secret {
            secret_env.push(SecretEnvVar {
                name: CREDENTIALS_ENV_VAR.to_string(),
                secret: secret.clone(),
                key: key.clone(),
            });
        }
        for entry in &config.pod.secret_env {
            upsert_secret_env(&mut secret_env, entry.clone());
        }
        if self.auth_proxy_url.is_some()
            && let Some(entry) = secret_env
                .iter()
                .find(|s| PROXY_FORBIDDEN_ENV.contains(&s.name.as_str()))
        {
            return Err(proxy_forbidden(&entry.name));
        }
        let proxied_secrets = self.merged_proxied_secrets(config);
        self.check_proxied_secrets(&proxied_secrets, &secret_env)?;

        let service_account = config
            .pod
            .service_account
            .clone()
            .or_else(|| self.service_account.clone());

        let runtime_class = config
            .pod
            .runtime_class
            .clone()
            .or_else(|| self.runtime_class.clone());
        if runtime_class
            .as_deref()
            .is_some_and(|c| c.trim().is_empty())
        {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: "runtime class must not be empty".to_string(),
            });
        }

        let (volumes, pvc_volumes): (&[_], &[_]) = if config.pod.without_provider_volumes {
            (&[], &[])
        } else {
            (&self.volumes, &self.pvc_volumes)
        };

        let mut read_only_volumes = self.read_only_volumes.clone();
        read_only_volumes.extend(config.pod.read_only_volumes.iter().cloned());

        let preset = config
            .pod
            .managed_settings
            .as_ref()
            .or(self.default_managed_settings.as_ref());
        let managed_settings_configmap = preset
            .map(|name| {
                let configmap = self.managed_settings_presets.get(name).cloned();
                configmap.ok_or_else(|| {
                    let known: Vec<&str> = self
                        .managed_settings_presets
                        .keys()
                        .map(String::as_str)
                        .collect();
                    AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!(
                            "unknown managed settings preset '{name}', known presets: [{}]",
                            known.join(", ")
                        ),
                    }
                })
            })
            .transpose()?;

        if let Some(key) = config.pod_labels.keys().find(|k| is_reserved_pod_label(k)) {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!(
                    "pod label '{key}' is reserved: ironflow sets it on every object it creates"
                ),
            });
        }
        let mut labels = self.pod_labels.clone();
        if let Some(profile) = &self.egress_profile {
            labels.insert(LABEL_EGRESS_PROFILE.to_string(), profile.clone());
        }
        labels.extend(config.pod_labels.clone());

        Ok(MergedPodInputs {
            volumes,
            pvc_volumes,
            secret_env,
            service_account,
            read_only_volumes,
            managed_settings_configmap,
            labels,
            runtime_class,
            proxied_secrets,
        })
    }

    /// Refuse every Claude credential the provider would hand to the pod
    /// itself while the auth proxy is set.
    fn check_no_proxy_credential(&self) -> Result<(), AgentError> {
        if self.oauth_credentials.is_some() {
            return Err(proxy_forbidden("oauth_credentials"));
        }
        if self.oauth_credentials_secret.is_some() {
            return Err(proxy_forbidden(CREDENTIALS_ENV_VAR));
        }
        let forbidden = self.env_vars.iter().find(|(key, value)| {
            PROXY_FORBIDDEN_ENV.contains(&key.as_str()) || value.starts_with("sk-ant")
        });
        match forbidden {
            Some((key, _)) => Err(proxy_forbidden(key)),
            None => Ok(()),
        }
    }

    /// The provider's proxied secrets, then the step's, a step entry
    /// replacing a provider entry with the same `env`.
    fn merged_proxied_secrets(&self, config: &AgentConfig) -> Vec<ProxiedSecret> {
        let mut secrets = self.proxied_secrets.clone();
        for secret in &config.proxied_secrets {
            upsert_proxied_secret(&mut secrets, secret.clone());
        }
        secrets
    }

    /// Refuse proxied secrets without the auth proxy, a proxied variable
    /// (`<env>` or `<env>_URL`) the pod already receives, and a
    /// [`SecretInjection::Basic`] secret when the pod already receives a git
    /// config variable. Names the variable, never a value.
    fn check_proxied_secrets(
        &self,
        secrets: &[ProxiedSecret],
        secret_env: &[SecretEnvVar],
    ) -> Result<(), AgentError> {
        if secrets.is_empty() {
            return Ok(());
        }
        if self.auth_proxy_url.is_none() {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: "proxied_secret requires auth_proxy".to_string(),
            });
        }
        let pod_env: Vec<&str> = self
            .env_vars
            .iter()
            .map(|(key, _)| key.as_str())
            .chain(secret_env.iter().map(|entry| entry.name.as_str()))
            .collect();
        let is_git_config = |name: &str| name.starts_with(GIT_CONFIG_PREFIX);
        let mut proxied: Vec<String> = Vec::new();
        for secret in secrets {
            for name in [secret.env.clone(), secret.url_env()] {
                let collides = PROXY_FORBIDDEN_ENV.contains(&name.as_str())
                    || name == NONESSENTIAL_TRAFFIC_ENV
                    || is_git_config(&name)
                    || pod_env.contains(&name.as_str())
                    || proxied.contains(&name);
                if collides {
                    return Err(AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!(
                            "proxied secret variable {name} is already set for the pod"
                        ),
                    });
                }
                proxied.push(name);
            }
        }
        let basic = secrets
            .iter()
            .any(|secret| matches!(secret.injection, SecretInjection::Basic { .. }));
        let git_config = pod_env.iter().find(|key| is_git_config(key));
        if basic && let Some(key) = git_config {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!(
                    "a Basic proxied secret sets the git config: the pod must not receive {key}"
                ),
            });
        }
        Ok(())
    }

    /// Plain environment variables of the pod: the provider's, plus the proxy
    /// URL and the opaque token when the auth proxy is set and a token was
    /// issued, then `<env>` (the token) and `<env>_URL` (`<proxy>/r`) of each
    /// proxied secret, then the git config of the [`SecretInjection::Basic`]
    /// secrets.
    ///
    /// For each non-wildcard host `h` of a Basic secret, git rewrites
    /// `https://h/` to `<proxy>/r/h/` and sends the token as the Basic
    /// password there; the proxy swaps it for the real secret.
    fn pod_env_vars(
        &self,
        proxy_token: Option<&str>,
        secrets: &[(ProxiedSecret, IssuedToken)],
    ) -> Vec<(String, String)> {
        let mut env = self.env_vars.clone();
        let Some(url) = &self.auth_proxy_url else {
            return env;
        };
        if let Some(token) = proxy_token {
            env.push((POD_BASE_URL_ENV.to_string(), url.clone()));
            env.push((POD_TOKEN_ENV.to_string(), token.to_string()));
            env.push((NONESSENTIAL_TRAFFIC_ENV.to_string(), "1".to_string()));
        }

        let relay = format!("{url}{RELAY_PREFIX}");
        let mut git_config: Vec<(String, String)> = Vec::new();
        for (secret, issued) in secrets {
            env.push((secret.env.clone(), issued.token.clone()));
            env.push((secret.url_env(), relay.clone()));
            let SecretInjection::Basic { username } = &secret.injection else {
                continue;
            };
            let basic = STANDARD.encode(format!("{username}:{}", issued.token));
            for host in &secret.hosts {
                let host = host.trim().to_ascii_lowercase();
                if host.starts_with("*.") {
                    continue;
                }
                let base = format!("{relay}/{host}/");
                git_config.push((format!("url.{base}.insteadOf"), format!("https://{host}/")));
                git_config.push((
                    format!("http.{base}.extraHeader"),
                    format!("Authorization: Basic {basic}"),
                ));
            }
        }
        if !git_config.is_empty() {
            let count = git_config.len().to_string();
            env.push((GIT_CONFIG_COUNT.to_string(), count));
            for (n, (key, value)) in git_config.into_iter().enumerate() {
                env.push((format!("{GIT_CONFIG_PREFIX}KEY_{n}"), key));
                env.push((format!("{GIT_CONFIG_PREFIX}VALUE_{n}"), value));
            }
        }
        env
    }

    /// Admin client of the auth proxy, `None` when no proxy is set.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when the proxy is set but no
    /// admin key is available.
    fn auth_proxy_client(&self) -> Result<Option<AuthProxyClient>, AgentError> {
        let Some(url) = &self.auth_proxy_url else {
            return Ok(None);
        };
        let key = self
            .auth_proxy_admin_key
            .clone()
            .or_else(|| var(ADMIN_KEY_ENV).ok().filter(|key| !key.is_empty()))
            .ok_or_else(|| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("auth_proxy requires {ADMIN_KEY_ENV}"),
            })?;
        Ok(Some(AuthProxyClient::new(url, &key)))
    }

    /// Revoke the opaque tokens of a step, best effort: their expiry is the backstop.
    async fn revoke_proxy_tokens(&self, ids: &[String]) {
        for id in ids {
            let short = id.get(..12).unwrap_or(id);
            let result = match self.auth_proxy_client() {
                Ok(Some(client)) => client.revoke(id).await.map_err(|e| e.to_string()),
                Ok(None) => Ok(()),
                Err(e) => Err(e.to_string()),
            };
            match result {
                Ok(()) => debug!(token = %short, "auth proxy token revoked"),
                Err(e) => warn!(
                    token = %short,
                    error = %e,
                    "auth proxy token revocation failed; it expires at expires-at"
                ),
            }
        }
    }

    /// Shell prefix filling `~/.claude`: the profiles, then the credentials,
    /// so that a profile cannot overwrite them.
    pub(super) fn home_setup_prefix(&self) -> String {
        let credentials = if self.oauth_credentials_secret.is_some() {
            build_credentials_from_env_prefix(CREDENTIALS_ENV_VAR)
        } else {
            build_credentials_prefix(self.oauth_credentials.as_deref())
        };
        let profiles = build_profile_copy_prefix(&self.claude_profiles);
        format!("{profiles}{credentials}")
    }

    /// Margin added to the timeout for the deadline and the expiry annotation.
    fn deadline_margin_or_default(&self) -> Duration {
        self.sandbox
            .as_ref()
            .map_or(DEFAULT_DEADLINE_MARGIN, |s| s.deadline_margin)
    }

    /// The `activeDeadlineSeconds` of the pod: the explicit value when set,
    /// else timeout + margin when sandboxed, else none.
    fn effective_deadline(&self) -> Option<Duration> {
        self.active_deadline_seconds.or_else(|| {
            self.sandbox
                .as_ref()
                .map(|s| self.timeout + s.deadline_margin)
        })
    }

    /// Delete the pods and prompt ConfigMaps left by a previous attempt of the
    /// same step of the same run, and wait until the pods are gone.
    ///
    /// Two branches of a parallel group with the same name would share the
    /// `ironflow.io/step` label, and one would delete the other's pod: the
    /// engine rejects such a group before creating any step.
    ///
    /// Returns the number of pods deleted.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when the pods cannot be listed
    /// or deleted, or are still terminating after
    /// [`previous_attempt_timeout`](Self::previous_attempt_timeout): a new
    /// agent never starts next to a live one.
    async fn delete_previous_attempt(
        &self,
        client: &Client,
        run_id: &str,
        step: &str,
    ) -> Result<usize, AgentError> {
        let selection = step_selection(run_id, step);
        let limit = self.previous_attempt_timeout;
        let deleted = delete_and_wait(client, &self.namespace, &selection, limit).await?;
        if deleted > 0 {
            info!(
                run_id = %run_id,
                step = %step,
                pods_deleted = deleted,
                "deleted pods of a previous attempt before retrying"
            );
        }
        Ok(deleted)
    }

    /// Delete the orphaned ironflow pods, Jobs, prompt ConfigMaps and expired
    /// environment claims of the provider's namespace: [`reap_orphans`] with
    /// the provider's cluster and namespace.
    ///
    /// # Errors
    ///
    /// Same as [`reap_orphans`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::AgentError> {
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1").namespace("ironflow-agents");
    /// let report = provider.reap_orphans().await?;
    /// println!("{} pods deleted", report.pods_deleted);
    /// # Ok(())
    /// # }
    /// ```
    pub async fn reap_orphans(&self) -> Result<ReapReport, AgentError> {
        reap_orphans(&self.cluster_config, &self.namespace).await
    }

    /// Spawn a background task calling [`reap_orphans`](Self::reap_orphans)
    /// every `interval`. Errors are logged, never fatal.
    ///
    /// Abort the returned handle to stop the reaper.
    ///
    /// # Panics
    ///
    /// Panics if `interval` is zero, or when called outside a Tokio runtime.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_core::providers::claude::K8sEphemeralProvider;
    ///
    /// # async fn example() {
    /// let provider = K8sEphemeralProvider::sandboxed("img:v1");
    /// let reaper = provider.spawn_orphan_reaper(Duration::from_secs(300));
    /// reaper.abort();
    /// # }
    /// ```
    pub fn spawn_orphan_reaper(&self, interval: Duration) -> JoinHandle<()> {
        assert!(
            !interval.is_zero(),
            "orphan reaper interval must be greater than zero"
        );
        // The task lives as long as the worker: keep only what a pass needs,
        // not the whole provider and its inline credentials.
        let cluster_config = self.cluster_config.clone();
        let namespace = self.namespace.clone();
        spawn(async move {
            let mut ticker = time::interval(interval);
            loop {
                ticker.tick().await;
                if let Err(e) = reap_orphans(&cluster_config, &namespace).await {
                    warn!(error = %e, "orphan reaping pass failed");
                }
            }
        })
    }

    /// Prepare config, create the K8s pod, and return handles for the wait phase.
    async fn create_pod(&self, config: &AgentConfig) -> Result<CreatedPod, AgentError> {
        claude_common::validate_prompt_size(config)?;
        let built = claude_common::build_command(config)?;
        let merged = self.merged_pod_inputs(config)?;

        let pod_name = generate_pod_name("claude-code");
        let creds_prefix = self.home_setup_prefix();

        let start = Instant::now();
        let client = create_client(&self.cluster_config).await?;
        let pods: Api<Pod> = Api::namespaced(client.clone(), &self.namespace);

        let mut prompt_configmap_name: Option<String> = None;
        let configmaps: Api<ConfigMap> = Api::namespaced(client.clone(), &self.namespace);

        let run_id = merged.labels.get(LABEL_RUN_ID);
        let step = merged.labels.get(LABEL_STEP);
        if let (Some(run_id), Some(step)) = (run_id, step) {
            self.delete_previous_attempt(&client, run_id, step).await?;
        }

        // Computed after the cleanup wait so it does not eat into the timeout.
        let lifetime = self.timeout + self.deadline_margin_or_default();
        let expires_at = now_unix()? + lifetime.as_secs();
        let mut annotations = BTreeMap::new();
        annotations.insert(LABEL_EXPIRES_AT.to_string(), expires_at.to_string());

        let trace_prefix = config
            .trace_context
            .as_ref()
            .map(|ctx| format!("export TRACEPARENT='{}'; ", ctx.to_traceparent()))
            .unwrap_or_default();

        let full_cmd = if let Some(ref prompt) = built.stdin_prompt {
            let cm_name = format!("{pod_name}-prompt");
            let mut cm_labels = BTreeMap::new();
            cm_labels.insert("app.kubernetes.io/managed-by", "ironflow");
            cm_labels.insert("app.kubernetes.io/component", "prompt-data");
            if let Some(root) = merged.labels.get(LABEL_ROOT_RUN_ID) {
                cm_labels.insert(LABEL_ROOT_RUN_ID, root.as_str());
            }
            if let (Some(run_id), Some(step)) = (run_id, step) {
                cm_labels.insert(LABEL_RUN_ID, run_id.as_str());
                cm_labels.insert(LABEL_STEP, step.as_str());
            }
            let cm: ConfigMap = from_value(json!({
                "apiVersion": "v1",
                "kind": "ConfigMap",
                "metadata": {
                    "name": &cm_name,
                    "namespace": &self.namespace,
                    "labels": cm_labels,
                    "annotations": &annotations
                },
                "data": {
                    PROMPT_CM_KEY: prompt
                }
            }))
            .map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to build prompt ConfigMap: {e}"),
            })?;

            configmaps
                .create(&PostParams::default(), &cm)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to create prompt ConfigMap: {e}"),
                })?;
            prompt_configmap_name = Some(cm_name);

            info!(
                pod = %pod_name,
                prompt_bytes = prompt.len(),
                "prompt too large for CLI args, using ConfigMap + stdin pipe"
            );

            let prompt_file = format!("{PROMPT_MOUNT_PATH}/{PROMPT_CM_KEY}");
            let claude_cmd = claude_common::build_shell_command(&self.claude_path, &built.args);
            let pipe_prefix = format!(
                "cat {} | ",
                claude_common::build_shell_command(&prompt_file, &[])
            );
            match (&self.working_dir, &config.working_dir) {
                (_, Some(dir)) | (Some(dir), None) => {
                    format!(
                        "{trace_prefix}{creds_prefix}cd {} && {pipe_prefix}{claude_cmd}",
                        claude_common::build_shell_command(dir, &[]),
                    )
                }
                (None, None) => format!("{trace_prefix}{creds_prefix}{pipe_prefix}{claude_cmd}"),
            }
        } else {
            let claude_cmd = claude_common::build_shell_command(&self.claude_path, &built.args);
            match (&self.working_dir, &config.working_dir) {
                (_, Some(dir)) | (Some(dir), None) => {
                    format!(
                        "{trace_prefix}{creds_prefix}cd {} && {}",
                        claude_common::build_shell_command(dir, &[]),
                        claude_cmd
                    )
                }
                (None, None) => format!("{trace_prefix}{creds_prefix}{claude_cmd}"),
            }
        };

        debug!(
            pod_name = %pod_name,
            namespace = %self.namespace,
            image = %self.image,
            model = %config.model,
            prompt_via_configmap = prompt_configmap_name.is_some(),
            "creating ephemeral K8s pod"
        );

        let issued = match self
            .issue_proxy_token(config, run_id, step, &pod_name, expires_at)
            .await
        {
            Ok(issued) => issued,
            Err(e) => {
                abort_launch_configmap(&configmaps, prompt_configmap_name.as_deref()).await;
                return Err(e);
            }
        };
        let claude_ids: Vec<String> = issued.iter().map(|t| t.id.clone()).collect();
        let proxied = &merged.proxied_secrets;
        let secrets = match self
            .issue_secret_tokens(proxied, run_id, step, &pod_name, expires_at)
            .await
        {
            Ok(secrets) => secrets,
            Err(e) => {
                self.revoke_proxy_tokens(&claude_ids).await;
                abort_launch_configmap(&configmaps, prompt_configmap_name.as_deref()).await;
                return Err(e);
            }
        };
        let env_vars = self.pod_env_vars(issued.as_ref().map(|t| t.token.as_str()), &secrets);
        let secret_ids = secrets.iter().map(|(_, t)| t.id.clone());
        let proxy_token_ids: Vec<String> = claude_ids.into_iter().chain(secret_ids).collect();

        let claims: Api<PersistentVolumeClaim> = Api::namespaced(client.clone(), &self.namespace);
        let environment = match self
            .prepare_environment(&claims, config, &merged.labels, lifetime)
            .await
        {
            Ok(environment) => environment,
            Err(e) => {
                self.revoke_proxy_tokens(&proxy_token_ids).await;
                abort_launch_configmap(&configmaps, prompt_configmap_name.as_deref()).await;
                return Err(e);
            }
        };
        let mut step_pvc_volumes = config.pod.pvc_volumes.clone();
        if let (Some(volume), Some(claim)) = (&self.environment, &environment) {
            step_pvc_volumes.push(environment_mount(volume, &claim.name));
        }

        let pod_spec = build_pod_spec(&PodConfig {
            name: &pod_name,
            image: &self.image,
            command: vec!["sh".to_string(), "-c".to_string(), full_cmd],
            namespace: &self.namespace,
            resources: &self.resources,
            service_account: merged.service_account.as_deref(),
            restart_policy: "Never",
            image_pull_policy: &self.image_pull_policy,
            env_vars: &env_vars,
            image_pull_secrets: &self.image_pull_secrets,
            extra_labels: &merged.labels,
            node_selector: &self.node_selector,
            tolerations: &self.tolerations,
            volumes: merged.volumes,
            pvc_volumes: merged.pvc_volumes,
            inputs: &config.inputs,
            input_init_image: &self.input_init_image,
            prompt_configmap: prompt_configmap_name.as_deref(),
            prompt_mount_path: PROMPT_MOUNT_PATH,
            hardening: PodHardening {
                sandbox: self.sandbox.as_ref(),
                secret_env: &merged.secret_env,
                read_only_volumes: &merged.read_only_volumes,
                step_pvc_volumes: &step_pvc_volumes,
                managed_settings_configmap: merged.managed_settings_configmap.as_deref(),
                claude_profiles: &self.claude_profiles,
                sessions_claim: self.sessions_claim.as_deref(),
                annotations: Some(&annotations),
            },
        });

        let created = match pod_spec {
            Ok(mut pod_spec) => {
                // Applied after build_pod_spec: the shared PodConfig builder does not
                // carry these fields, so they are set on the built pod here.
                apply_active_deadline_seconds(&mut pod_spec, self.effective_deadline());
                apply_runtime_class(&mut pod_spec, merged.runtime_class.as_deref());
                pods.create(&PostParams::default(), &pod_spec)
                    .await
                    .map_err(|e| AgentError::ProcessFailed {
                        exit_code: -1,
                        stderr: format!("failed to create K8s pod: {e}"),
                    })
            }
            Err(e) => Err(e),
        };
        if let Err(e) = created {
            // No pod runs with the token: drop it now rather than at expiry.
            self.revoke_proxy_tokens(&proxy_token_ids).await;
            abort_launch_configmap(&configmaps, prompt_configmap_name.as_deref()).await;
            if let Some(claim) = &environment {
                abort_environment_claim(&claims, claim).await;
            }
            return Err(e);
        }

        Ok(CreatedPod {
            pods,
            pod_name,
            start,
            prompt_configmap: prompt_configmap_name,
            configmaps: Some(configmaps),
            proxy_token_ids,
            environment_id: environment.map(|claim| claim.name),
        })
    }

    /// Issue the auth proxy token of a pod, `None` when no proxy is set.
    ///
    /// The token is bound to the run and step labels (the pod name and
    /// `agent` for an invocation outside a run) and expires with the pod.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when no admin key or credential
    /// is available, or when the proxy refuses or cannot be reached.
    async fn issue_proxy_token(
        &self,
        config: &AgentConfig,
        run_id: Option<&String>,
        step: Option<&String>,
        pod_name: &str,
        expires_at: u64,
    ) -> Result<Option<IssuedToken>, AgentError> {
        let Some(client) = self.auth_proxy_client()? else {
            return Ok(None);
        };
        let credential = resolve_credential(config.account.as_ref(), |k| var(k).ok());
        let credential = credential.map_err(auth_proxy_error)?;
        let (run_id, step) = grant_scope(run_id, step, pod_name);
        let request = TokenRequest {
            run_id,
            step,
            expires_at,
            credential: credential.into(),
        };
        let issued = client.issue(&request).await.map_err(auth_proxy_error)?;
        info!(
            token = %issued.short_id(),
            pod = %pod_name,
            run_id = %request.run_id,
            step = %request.step,
            "auth proxy token issued"
        );
        Ok(Some(issued))
    }

    /// Issue one auth proxy token per proxied secret of a pod, none when no
    /// proxy is set.
    ///
    /// Each token is bound to the same run and step labels as the Claude
    /// token and expires with the pod. When one issuance fails, the tokens
    /// already issued are revoked before the error is returned.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when no admin key is available,
    /// when a secret is invalid, or when the proxy refuses or cannot be
    /// reached.
    async fn issue_secret_tokens(
        &self,
        secrets: &[ProxiedSecret],
        run_id: Option<&String>,
        step: Option<&String>,
        pod_name: &str,
        expires_at: u64,
    ) -> Result<Vec<(ProxiedSecret, IssuedToken)>, AgentError> {
        if secrets.is_empty() {
            return Ok(Vec::new());
        }
        let Some(client) = self.auth_proxy_client()? else {
            return Ok(Vec::new());
        };
        let (run_id, step) = grant_scope(run_id, step, pod_name);
        let mut issued: Vec<(ProxiedSecret, IssuedToken)> = Vec::with_capacity(secrets.len());
        for secret in secrets {
            let result = match secret.to_credential() {
                Ok(credential) => {
                    let request = TokenRequest {
                        run_id: run_id.clone(),
                        step: step.clone(),
                        expires_at,
                        credential: credential.into(),
                    };
                    client.issue(&request).await
                }
                Err(e) => Err(e),
            };
            match result {
                Ok(token) => {
                    info!(
                        token = %token.short_id(),
                        secret = %secret.name,
                        pod = %pod_name,
                        run_id = %run_id,
                        step = %step,
                        "auth proxy secret token issued"
                    );
                    issued.push((secret.clone(), token));
                }
                Err(e) => {
                    let ids: Vec<String> = issued.into_iter().map(|(_, t)| t.id).collect();
                    self.revoke_proxy_tokens(&ids).await;
                    return Err(auth_proxy_error(e));
                }
            }
        }
        Ok(issued)
    }

    /// Create or resume the environment claim of a pod, `None` when the
    /// provider has no [`environment_volume`](Self::environment_volume).
    ///
    /// A new claim expires `ttl` (at least `lifetime`) from now; a resumed
    /// claim gets its expiry pushed to the same point.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when the claim cannot be
    /// created, when the claim to resume does not exist, is not an ironflow
    /// environment or is being deleted, or when its expiry cannot be updated.
    async fn prepare_environment(
        &self,
        claims: &Api<PersistentVolumeClaim>,
        config: &AgentConfig,
        pod_labels: &BTreeMap<String, String>,
        lifetime: Duration,
    ) -> Result<Option<EnvironmentClaim>, AgentError> {
        let Some(volume) = &self.environment else {
            return Ok(None);
        };
        let claim = environment_claim_for(config.resume_environment_id.as_deref())?;
        let expires_at = now_unix()? + volume.ttl.max(lifetime).as_secs();

        if claim.created {
            let pvc = build_environment_claim(
                volume,
                &claim.name,
                &self.namespace,
                pod_labels,
                expires_at,
            )?;
            claims
                .create(&PostParams::default(), &pvc)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to create environment claim '{}': {e}", claim.name),
                })?;
            info!(environment = %claim.name, "persistent environment created");
            return Ok(Some(claim));
        }

        let existing =
            claims
                .get_opt(&claim.name)
                .await
                .map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to read environment '{}': {e}", claim.name),
                })?;
        if !existing.as_ref().is_some_and(is_live_environment_claim) {
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("environment '{}' not found or expired", claim.name),
            });
        }
        let patch = json!({
            "metadata": { "annotations": { LABEL_EXPIRES_AT: expires_at.to_string() } }
        });
        claims
            .patch(&claim.name, &PatchParams::default(), &Patch::Merge(&patch))
            .await
            .map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to extend environment '{}': {e}", claim.name),
            })?;
        info!(environment = %claim.name, "persistent environment resumed");
        Ok(Some(claim))
    }

    /// Read pod phase after completion, delete the pod, and parse the output.
    fn finalize_pod(
        &self,
        logs: &str,
        pod_phase: &str,
        timed_out: bool,
        pod_name: &str,
        config: &AgentConfig,
        start: Instant,
    ) -> Result<AgentOutput, AgentError> {
        rate_limit_event::record_rate_limits(config, logs);

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
                logs,
                "",
                config,
                duration_ms,
                "ephemeral k8s",
            );
        }

        debug!(stdout_len = logs.len(), "ephemeral claude pod completed");
        claude_common::parse_output(logs, config, duration_ms)
    }
}

impl K8sEphemeralProvider {
    /// Wait for a created pod to complete, read its logs, delete it and
    /// parse the output.
    async fn run_created(
        &self,
        config: &AgentConfig,
        created: &CreatedPod,
    ) -> Result<AgentOutput, AgentError> {
        let CreatedPod {
            pods,
            pod_name,
            start,
            prompt_configmap,
            configmaps,
            environment_id,
            ..
        } = created;

        // Wait for pod to complete
        let wait_result = time::timeout(
            self.timeout,
            await_condition(pods.clone(), pod_name, is_pod_completed()),
        )
        .await;

        let timed_out = wait_result.is_err();
        let pod_phase = if timed_out {
            "TimedOut".to_string()
        } else {
            let condition_result = wait_result.expect("timeout already handled").map_err(|e| {
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

        let logs = pods
            .logs(pod_name, &LogParams::default())
            .await
            .unwrap_or_default();

        let _ = pods.delete(pod_name, &DeleteParams::default()).await;
        if let (Some(cm_name), Some(cm_api)) = (prompt_configmap, configmaps) {
            let _ = cm_api.delete(cm_name, &DeleteParams::default()).await;
        }

        let output = self.finalize_pod(&logs, &pod_phase, timed_out, pod_name, config, *start)?;
        Ok(with_environment(output, environment_id.as_deref()))
    }

    /// Stream the logs of a created pod into `log_sink` until it completes,
    /// delete it and parse the output.
    async fn run_created_with_logs(
        &self,
        config: &AgentConfig,
        created: &CreatedPod,
        log_sink: Arc<dyn LogSink>,
    ) -> Result<AgentOutput, AgentError> {
        let CreatedPod {
            pods,
            pod_name,
            start,
            prompt_configmap,
            configmaps,
            environment_id,
            ..
        } = created;

        let ready_result = time::timeout(
            self.timeout,
            await_condition(pods.clone(), pod_name, is_pod_running_or_terminal()),
        )
        .await;

        if let Err(_elapsed) = ready_result {
            let _ = pods.delete(pod_name, &DeleteParams::default()).await;
            if let (Some(cm_name), Some(cm_api)) = (prompt_configmap, configmaps) {
                let _ = cm_api.delete(cm_name, &DeleteParams::default()).await;
            }
            warn!(timeout = ?self.timeout, pod = %pod_name, "K8s pod timed out waiting for Running");
            return Err(AgentError::Timeout {
                limit: self.timeout,
            });
        }

        if let Err(e) = ready_result.expect("timeout already handled") {
            let _ = pods.delete(pod_name, &DeleteParams::default()).await;
            if let (Some(cm_name), Some(cm_api)) = (prompt_configmap, configmaps) {
                let _ = cm_api.delete(cm_name, &DeleteParams::default()).await;
            }
            return Err(AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed waiting for pod to start: {e}"),
            });
        }

        let phase_after_ready = pods
            .get(pod_name)
            .await
            .ok()
            .and_then(|p| p.status)
            .and_then(|s| s.phase);
        let already_terminal = phase_after_ready.as_deref().is_some_and(is_terminal_phase);

        let mut accumulated = String::new();
        let mut timed_out = false;

        if already_terminal {
            debug!(pod = %pod_name, phase = ?phase_after_ready, "pod already terminal, skipping log stream");
            accumulated = pods
                .logs(pod_name, &LogParams::default())
                .await
                .unwrap_or_default();
            for line in accumulated.lines() {
                log_sink.log("stdout", line);
            }
        } else {
            let log_params = LogParams {
                follow: true,
                ..Default::default()
            };

            const MAX_ACCUMULATED_BYTES: usize = 50 * 1024 * 1024;
            let mut truncated = false;

            let completion_notify = Arc::new(tokio::sync::Notify::new());

            let watcher_handle = {
                let pods = pods.clone();
                let pod_name = pod_name.to_string();
                let notify = completion_notify.clone();
                tokio::spawn(async move {
                    let _ = await_condition(pods, &pod_name, is_pod_completed()).await;
                    notify.notify_waiters();
                })
            };

            let stream_result = time::timeout(
                self.timeout,
                async {
                    match pods.log_stream(pod_name, &log_params).await {
                        Ok(stream) => {
                            let mut lines = stream.lines();
                            loop {
                                tokio::select! {
                                    line_result = lines.try_next() => {
                                        match line_result {
                                            Ok(Some(line)) => {
                                                log_sink.log("stdout", &line);
                                                if !truncated {
                                                    if accumulated.len() + line.len() + 1
                                                        > MAX_ACCUMULATED_BYTES
                                                    {
                                                        truncated = true;
                                                        warn!(pod = %pod_name, "log accumulation cap reached, further output will only be streamed");
                                                    } else {
                                                        accumulated.push_str(&line);
                                                        accumulated.push('\n');
                                                    }
                                                }
                                            }
                                            Ok(None) => break,
                                            Err(_) => break,
                                        }
                                    }
                                    _ = completion_notify.notified() => {
                                        debug!(pod = %pod_name, "pod completed, draining remaining log lines");
                                        while let Ok(Some(line)) = time::timeout(
                                            Duration::from_secs(2),
                                            lines.try_next(),
                                        ).await.unwrap_or(Ok(None)) {
                                            log_sink.log("stdout", &line);
                                            if !truncated {
                                                if accumulated.len() + line.len() + 1
                                                    > MAX_ACCUMULATED_BYTES
                                                {
                                                    truncated = true;
                                                } else {
                                                    accumulated.push_str(&line);
                                                    accumulated.push('\n');
                                                }
                                            }
                                        }
                                        break;
                                    }
                                }
                            }
                            Ok(())
                        }
                        Err(e) => Err(e),
                    }
                },
            )
            .await;

            watcher_handle.abort();
            timed_out = stream_result.is_err();
            if let Ok(Err(e)) = stream_result {
                warn!(pod = %pod_name, error = %e, "failed to open log stream, falling back to batch read");
                let _ = time::timeout(
                    self.timeout,
                    await_condition(pods.clone(), pod_name, is_pod_completed()),
                )
                .await;

                accumulated = pods
                    .logs(pod_name, &LogParams::default())
                    .await
                    .unwrap_or_default();
                for line in accumulated.lines() {
                    log_sink.log("stdout", line);
                }
            }
        }

        let pod_phase = if already_terminal {
            phase_after_ready.unwrap_or_else(|| "Unknown".to_string())
        } else {
            match pods.get(pod_name).await {
                Ok(pod) => pod
                    .status
                    .and_then(|s| s.phase)
                    .unwrap_or_else(|| "Unknown".to_string()),
                Err(_) => "Unknown".to_string(),
            }
        };

        let _ = pods.delete(pod_name, &DeleteParams::default()).await;
        if let (Some(cm_name), Some(cm_api)) = (prompt_configmap, configmaps) {
            let _ = cm_api.delete(cm_name, &DeleteParams::default()).await;
        }

        let output = self.finalize_pod(
            &accumulated,
            &pod_phase,
            timed_out,
            pod_name,
            config,
            *start,
        )?;
        Ok(with_environment(output, environment_id.as_deref()))
    }
}

impl AgentProvider for K8sEphemeralProvider {
    /// Delete every pod, `JobRun` Job and prompt ConfigMap labelled with the
    /// run (`ironflow.io/run-id`) or with the run as the root of a
    /// sub-workflow (`ironflow.io/root-run-id`), and wait up to
    /// [`previous_attempt_timeout`](Self::previous_attempt_timeout) until the
    /// pods are gone. With an auth proxy, every token of the run is revoked
    /// too, best effort.
    fn release_run<'a>(&'a self, run_id: &'a str) -> ReleaseFuture<'a> {
        let limit = self.previous_attempt_timeout;
        Box::pin(async move {
            let cleanup = release_run(&self.cluster_config, &self.namespace, run_id, limit).await;
            if let Ok(Some(client)) = self.auth_proxy_client()
                && let Err(e) = client.revoke_run(run_id).await
            {
                warn!(
                    run_id = %run_id,
                    error = %e,
                    "auth proxy token revocation of the run failed; they expire on their own"
                );
            }
            cleanup
        })
    }

    /// Provider Accounts are injected only through the auth proxy: without
    /// it, the pod credential comes from the provider settings.
    fn account_kind(&self) -> Option<&'static str> {
        self.auth_proxy_url
            .as_ref()
            .map(|_| ClaudeSubscriptionKind::ID)
    }

    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            let created = self.create_pod(config).await?;
            let result = self.run_created(config, &created).await;
            self.revoke_proxy_tokens(&created.proxy_token_ids).await;
            result
        })
    }

    fn invoke_with_logs<'a>(
        &'a self,
        config: &'a AgentConfig,
        log_sink: Arc<dyn LogSink>,
    ) -> InvokeFuture<'a> {
        Box::pin(async move {
            let effective_config;
            let config = if !config.verbose {
                debug!("forcing verbose=true for log streaming (stream-json output required)");
                effective_config = config.clone().verbose(true);
                &effective_config
            } else {
                config
            };

            let created = self.create_pod(config).await?;
            let result = self.run_created_with_logs(config, &created, log_sink).await;
            self.revoke_proxy_tokens(&created.proxy_token_ids).await;
            result
        })
    }
}

#[cfg(test)]
mod label_tests;

#[cfg(test)]
mod tests {
    use serde_json::{Value, to_value};

    use super::super::toleration::{TolerationEffect, TolerationOperator};
    use super::*;
    use crate::provider::{StorageUnit, VolumeSize};

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

    #[test]
    fn ephemeral_provider_volume_builder() {
        let provider = K8sEphemeralProvider::new("img:v1")
            .volume("/tmp/worktrees", "/data/worktrees")
            .volume("/tmp/repos", "/data/repos");
        assert_eq!(provider.volumes.len(), 2);
        assert_eq!(
            provider.volumes[0],
            ("/tmp/worktrees".to_string(), "/data/worktrees".to_string())
        );
        assert_eq!(
            provider.volumes[1],
            ("/tmp/repos".to_string(), "/data/repos".to_string())
        );
    }

    #[test]
    fn ephemeral_provider_volumes_default_empty() {
        let provider = K8sEphemeralProvider::new("img:v1");
        assert!(provider.volumes.is_empty());
    }

    #[test]
    fn ephemeral_provider_pvc_volume_builder() {
        let provider = K8sEphemeralProvider::new("img:v1")
            .pvc_volume("jarvis-repos", "/data/repos")
            .pvc_volume("jarvis-worktrees", "/data/worktrees");
        assert_eq!(provider.pvc_volumes.len(), 2);
        assert_eq!(
            provider.pvc_volumes[0],
            ("jarvis-repos".to_string(), "/data/repos".to_string())
        );
        assert_eq!(
            provider.pvc_volumes[1],
            (
                "jarvis-worktrees".to_string(),
                "/data/worktrees".to_string()
            )
        );
    }

    #[test]
    fn ephemeral_provider_pvc_volumes_default_empty() {
        let provider = K8sEphemeralProvider::new("img:v1");
        assert!(provider.pvc_volumes.is_empty());
    }

    #[test]
    fn ephemeral_provider_node_selector_default_empty() {
        let provider = K8sEphemeralProvider::new("img:v1");
        assert!(provider.node_selector.is_empty());
    }

    #[test]
    fn ephemeral_provider_node_selector_accumulates() {
        let provider = K8sEphemeralProvider::new("img:v1")
            .node_selector("kubernetes.io/hostname", "ryzen1")
            .node_selector("workload", "agent");
        assert_eq!(provider.node_selector.len(), 2);
        assert_eq!(provider.node_selector["kubernetes.io/hostname"], "ryzen1");
        assert_eq!(provider.node_selector["workload"], "agent");
    }

    #[test]
    fn ephemeral_provider_toleration_default_empty() {
        let provider = K8sEphemeralProvider::new("img:v1");
        assert!(provider.tolerations.is_empty());
    }

    #[test]
    fn ephemeral_provider_toleration_accumulates() {
        let provider = K8sEphemeralProvider::new("img:v1")
            .toleration(K8sToleration {
                key: "dedicated".to_string(),
                operator: TolerationOperator::Equal,
                value: Some("worker".to_string()),
                effect: TolerationEffect::NoSchedule,
                toleration_seconds: None,
            })
            .toleration(K8sToleration {
                key: "gpu".to_string(),
                operator: TolerationOperator::Exists,
                value: None,
                effect: TolerationEffect::NoExecute,
                toleration_seconds: None,
            });
        assert_eq!(provider.tolerations.len(), 2);
        assert_eq!(provider.tolerations[0].key, "dedicated");
        assert_eq!(provider.tolerations[1].operator, TolerationOperator::Exists);
    }

    #[test]
    fn ephemeral_provider_active_deadline_default_and_builder() {
        let default = K8sEphemeralProvider::new("img:v1");
        assert!(default.active_deadline_seconds.is_none());

        let provider =
            K8sEphemeralProvider::new("img:v1").active_deadline_seconds(Duration::from_secs(900));
        assert_eq!(
            provider.active_deadline_seconds,
            Some(Duration::from_secs(900))
        );
    }

    #[test]
    fn apply_active_deadline_sets_field_in_seconds() {
        let mut pod: Pod =
            from_value(json!({"spec": {"containers": []}})).expect("valid minimal pod");
        apply_active_deadline_seconds(&mut pod, Some(Duration::from_secs(600)));
        assert_eq!(pod.spec.unwrap().active_deadline_seconds, Some(600));
    }

    #[test]
    fn apply_active_deadline_none_leaves_field_absent() {
        let mut pod: Pod =
            from_value(json!({"spec": {"containers": []}})).expect("valid minimal pod");
        apply_active_deadline_seconds(&mut pod, None);
        assert_eq!(pod.spec.unwrap().active_deadline_seconds, None);
    }

    #[test]
    fn k8s_runtime_class_default_is_none_and_builder_stores_value() {
        assert!(K8sEphemeralProvider::new("img:v1").runtime_class.is_none());
        let provider = K8sEphemeralProvider::new("img:v1").runtime_class("gvisor");
        assert_eq!(provider.runtime_class.as_deref(), Some("gvisor"));
    }

    #[test]
    #[should_panic(expected = "runtime class must not be empty")]
    fn k8s_runtime_class_blank_builder_panics() {
        let _ = K8sEphemeralProvider::new("img:v1").runtime_class("  ");
    }

    #[test]
    fn k8s_merged_step_runtime_class_overrides_provider() {
        let provider = K8sEphemeralProvider::new("img:v1").runtime_class("gvisor");
        let step = AgentConfig::new("hi").runtime_class("kata");
        assert_eq!(
            merge(&provider, &step).runtime_class.as_deref(),
            Some("kata")
        );
        assert_eq!(
            merge(&provider, &AgentConfig::new("hi"))
                .runtime_class
                .as_deref(),
            Some("gvisor")
        );
        let plain = K8sEphemeralProvider::new("img:v1");
        assert_eq!(merge(&plain, &AgentConfig::new("hi")).runtime_class, None);
    }

    #[test]
    fn k8s_merged_blank_step_runtime_class_is_an_error() {
        let provider = K8sEphemeralProvider::new("img:v1");
        let config = AgentConfig::new("hi").runtime_class(" ");
        let err = err_text(provider.merged_pod_inputs(&config));
        assert!(err.contains("runtime class must not be empty"), "{err}");
    }

    #[test]
    fn k8s_apply_runtime_class_sets_runtime_class_name() {
        let mut pod: Pod =
            from_value(json!({"spec": {"containers": []}})).expect("valid minimal pod");
        apply_runtime_class(&mut pod, Some("gvisor"));
        assert_eq!(
            pod.spec.unwrap().runtime_class_name.as_deref(),
            Some("gvisor")
        );
    }

    #[test]
    fn k8s_apply_runtime_class_none_leaves_field_absent() {
        let mut pod: Pod =
            from_value(json!({"spec": {"containers": []}})).expect("valid minimal pod");
        apply_runtime_class(&mut pod, None);
        assert_eq!(pod.spec.unwrap().runtime_class_name, None);
    }

    // ── Sandbox ─────────────────────────────────────────────────────

    fn err_text(result: Result<MergedPodInputs<'_>, AgentError>) -> String {
        result.unwrap_err().to_string()
    }

    fn merge<'a>(provider: &'a K8sEphemeralProvider, config: &AgentConfig) -> MergedPodInputs<'a> {
        provider.merged_pod_inputs(config).unwrap()
    }

    #[test]
    fn sandboxed_defaults() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1");
        let sandbox = provider.sandbox.as_ref().expect("sandbox set");
        assert_eq!(sandbox, &SandboxSettings::default());
        assert_eq!(sandbox.deadline_margin, Duration::from_secs(60));
        assert_eq!(provider.image, "img:v1");
        assert_eq!(provider.previous_attempt_timeout, Duration::from_secs(60));
        assert!(K8sEphemeralProvider::new("img:v1").sandbox.is_none());
    }

    #[test]
    fn sandboxed_relaxation_builders() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .allow_writable_root()
            .home_size_limit("4Gi")
            .tmp_size_limit("2Gi")
            .deadline_margin(Duration::from_secs(120))
            .run_as_user(20000);
        let sandbox = provider.sandbox.unwrap();
        assert!(sandbox.writable_root);
        assert_eq!(sandbox.home_size_limit, "4Gi");
        assert_eq!(sandbox.tmp_size_limit, "2Gi");
        assert_eq!(sandbox.deadline_margin, Duration::from_secs(120));
        assert_eq!(sandbox.run_as_user, 20000);
    }

    #[test]
    #[should_panic(expected = "allow_writable_root requires a provider built with")]
    fn allow_writable_root_on_non_sandboxed_panics() {
        let _ = K8sEphemeralProvider::new("img:v1").allow_writable_root();
    }

    #[test]
    #[should_panic(expected = "home_size_limit requires a provider built with")]
    fn home_size_limit_on_non_sandboxed_panics() {
        let _ = K8sEphemeralProvider::new("img:v1").home_size_limit("1Gi");
    }

    #[test]
    fn sessions_volume_builder_stores_claim_for_session_id() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1").sessions_volume("claude-sessions");
        assert_eq!(provider.sessions_claim.as_deref(), Some("claude-sessions"));
        assert!(
            K8sEphemeralProvider::sandboxed("img:v1")
                .sessions_claim
                .is_none()
        );
    }

    #[test]
    #[should_panic(
        expected = "sessions_volume requires a provider built with K8sEphemeralProvider::sandboxed"
    )]
    fn sessions_volume_on_non_sandboxed_panics() {
        let _ = K8sEphemeralProvider::new("img:v1").sessions_volume("claude-sessions");
    }

    #[test]
    #[should_panic(expected = "sessions_volume claim name must not be empty")]
    fn sessions_volume_empty_claim_panics() {
        let _ = K8sEphemeralProvider::sandboxed("img:v1").sessions_volume("");
    }

    #[test]
    #[should_panic(expected = "run_as_user must be greater than 0")]
    fn run_as_user_zero_panics() {
        let _ = K8sEphemeralProvider::sandboxed("img:v1").run_as_user(0);
    }

    #[test]
    fn oauth_token_from_secret_builder() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .oauth_token_from_secret("claude-oauth", "token");
        assert_eq!(provider.secret_env.len(), 1);
        assert_eq!(provider.secret_env[0].name, "CLAUDE_CODE_OAUTH_TOKEN");
        assert_eq!(provider.secret_env[0].secret, "claude-oauth");
        assert_eq!(provider.secret_env[0].key, "token");
    }

    #[test]
    fn oauth_credentials_from_secret_adds_credentials_env() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .oauth_credentials_from_secret("claude-credentials", "credentials.json");
        let (secret, key) = provider.oauth_credentials_secret.clone().unwrap();
        assert_eq!(secret, "claude-credentials");
        assert_eq!(key, "credentials.json");
        let merged = merge(&provider, &AgentConfig::new("hi"));
        assert_eq!(merged.secret_env.len(), 1);
        assert_eq!(merged.secret_env[0].name, CREDENTIALS_ENV_VAR);
        assert_eq!(merged.secret_env[0].secret, "claude-credentials");
    }

    #[test]
    fn env_from_secret_replaces_same_var() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .env_from_secret("TOKEN", "a", "k")
            .env_from_secret("TOKEN", "b", "k");
        assert_eq!(provider.secret_env.len(), 1);
        assert_eq!(provider.secret_env[0].secret, "b");
    }

    #[test]
    fn managed_settings_preset_map() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .managed_settings_preset("locked", "claude-managed-locked")
            .managed_settings_preset("readonly", "claude-managed-readonly")
            .default_managed_settings("locked");
        assert_eq!(provider.managed_settings_presets.len(), 2);
        assert_eq!(
            provider.managed_settings_presets["readonly"],
            "claude-managed-readonly"
        );
        assert_eq!(provider.default_managed_settings.as_deref(), Some("locked"));
    }

    #[test]
    fn other_sandbox_builders() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .read_only_pvc("repos", "/data/repos")
            .claude_profile_configmap("claude-profile")
            .egress_profile("anthropic-only")
            .previous_attempt_timeout(Duration::from_secs(5));
        assert_eq!(provider.read_only_volumes.len(), 1);
        assert_eq!(provider.read_only_volumes[0].mount_path, "/data/repos");
        assert_eq!(provider.claude_profiles[0].configmap, "claude-profile");
        assert_eq!(provider.egress_profile.as_deref(), Some("anthropic-only"));
        assert_eq!(provider.previous_attempt_timeout, Duration::from_secs(5));
    }

    #[test]
    fn merged_step_secret_overrides_provider_secret() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .env_from_secret("TOKEN", "provider-secret", "k")
            .env_from_secret("OTHER", "other", "k");
        let config = AgentConfig::new("hi")
            .env_from_secret("TOKEN", "step-secret", "k2")
            .env_from_secret("STEP_ONLY", "step", "k");
        let merged = merge(&provider, &config);
        assert_eq!(merged.secret_env.len(), 3);
        // The step entry replaces the provider entry in place.
        let token = &merged.secret_env[0];
        assert_eq!(token.name, "TOKEN");
        assert_eq!(token.secret, "step-secret");
        assert_eq!(token.key, "k2");
    }

    #[test]
    fn merged_step_service_account_overrides_provider() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1").service_account("provider-sa");
        let merged = merge(&provider, &AgentConfig::new("hi"));
        assert_eq!(merged.service_account.as_deref(), Some("provider-sa"));

        let config = AgentConfig::new("hi").service_account("step-sa");
        let merged = merge(&provider, &config);
        assert_eq!(merged.service_account.as_deref(), Some("step-sa"));
    }

    fn provider_with_volumes() -> K8sEphemeralProvider {
        K8sEphemeralProvider::sandboxed("img:v1")
            .volume("/srv/work", "/data/work")
            .pvc_volume("repos", "/data/repos")
    }

    #[test]
    fn merged_provider_volumes_kept_without_flag() {
        let provider = provider_with_volumes();
        let config = AgentConfig::new("hi");
        let merged = merge(&provider, &config);
        assert_eq!(merged.volumes.len(), 1);
        assert_eq!(merged.pvc_volumes.len(), 1);
    }

    #[test]
    fn merged_without_provider_volumes_drops_them() {
        let provider = provider_with_volumes();
        let config = AgentConfig::new("hi").without_provider_volumes();
        let merged = merge(&provider, &config);
        assert!(merged.volumes.is_empty());
        assert!(merged.pvc_volumes.is_empty());
    }

    #[test]
    fn merged_step_pvc_volumes_keep_provider_ones() {
        let provider = provider_with_volumes();
        let config = AgentConfig::new("hi").pvc_volume("scratch", "/scratch", Some("a"), false);
        let merged = merge(&provider, &config);
        assert_eq!(
            merged.pvc_volumes,
            [("repos".to_string(), "/data/repos".to_string())]
        );
        assert_eq!(merged.volumes.len(), 1);
    }

    #[test]
    fn pod_spec_without_provider_volumes_keeps_home_and_tmp() {
        let provider = provider_with_volumes().read_only_pvc("ro-claim", "/data/ro");
        let config = AgentConfig::new("hi")
            .without_provider_volumes()
            .pvc_volume("scratch", "/data/work", None, false);
        let merged = merge(&provider, &config);
        let pod = to_value(
            build_pod_spec(&PodConfig {
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
                volumes: merged.volumes,
                pvc_volumes: merged.pvc_volumes,
                inputs: &[],
                input_init_image: DEFAULT_INPUT_INIT_IMAGE,
                prompt_configmap: None,
                prompt_mount_path: "",
                hardening: PodHardening {
                    sandbox: provider.sandbox.as_ref(),
                    read_only_volumes: &merged.read_only_volumes,
                    step_pvc_volumes: &config.pod.pvc_volumes,
                    ..PodHardening::default()
                },
            })
            .unwrap(),
        )
        .unwrap();
        let names: Vec<&str> = pod["spec"]["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v["name"].as_str().unwrap())
            .collect();
        assert!(!names.contains(&"pvc-0"));
        assert!(!names.contains(&"vol-0"));
        assert!(names.contains(&"step-pvc-0"));
        assert!(names.contains(&"ro-0"));
        assert!(names.contains(&"ironflow-home"));
        assert!(names.contains(&"ironflow-tmp"));
    }

    #[test]
    fn merged_read_only_volumes_provider_then_step() {
        let provider =
            K8sEphemeralProvider::sandboxed("img:v1").read_only_pvc("repos", "/data/repos");
        let config = AgentConfig::new("hi").read_only_config_map("cm", "/data/cm");
        let merged = merge(&provider, &config);
        let paths: Vec<&str> = merged
            .read_only_volumes
            .iter()
            .map(|v| v.mount_path.as_str())
            .collect();
        assert_eq!(paths, vec!["/data/repos", "/data/cm"]);
    }

    #[test]
    fn merged_unknown_preset_is_an_error() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .managed_settings_preset("locked", "claude-managed-locked");
        let config = AgentConfig::new("hi").managed_settings("typo");
        let err = err_text(provider.merged_pod_inputs(&config));
        assert!(
            err.contains("unknown managed settings preset 'typo'"),
            "{err}"
        );
        assert!(err.contains("locked"), "{err}");
    }

    #[test]
    fn merged_unknown_default_preset_is_an_error() {
        let provider =
            K8sEphemeralProvider::sandboxed("img:v1").default_managed_settings("missing");
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(
            err.contains("unknown managed settings preset 'missing'"),
            "{err}"
        );
    }

    #[test]
    fn merged_default_preset_used_when_step_has_none() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .managed_settings_preset("locked", "claude-managed-locked")
            .managed_settings_preset("readonly", "claude-managed-readonly")
            .default_managed_settings("locked");
        let merged = merge(&provider, &AgentConfig::new("hi"));
        assert_eq!(
            merged.managed_settings_configmap.as_deref(),
            Some("claude-managed-locked")
        );

        let config = AgentConfig::new("hi").managed_settings("readonly");
        let merged = merge(&provider, &config);
        assert_eq!(
            merged.managed_settings_configmap.as_deref(),
            Some("claude-managed-readonly")
        );
    }

    #[test]
    fn merged_no_preset_means_no_managed_settings() {
        let provider = K8sEphemeralProvider::new("img:v1");
        let merged = merge(&provider, &AgentConfig::new("hi"));
        assert!(merged.managed_settings_configmap.is_none());
    }

    #[test]
    fn merged_step_egress_label_overrides_provider() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1").egress_profile("anthropic-only");
        let merged = merge(&provider, &AgentConfig::new("hi"));
        assert_eq!(merged.labels[LABEL_EGRESS_PROFILE], "anthropic-only");

        let config = AgentConfig::new("hi").egress_profile("gitlab");
        let merged = merge(&provider, &config);
        assert_eq!(merged.labels[LABEL_EGRESS_PROFILE], "gitlab");
    }

    #[test]
    fn merged_labels_carry_run_scope() {
        let provider = K8sEphemeralProvider::new("img:v1").pod_label("team", "infra");
        let config = AgentConfig::new("hi").run_scope("run-1", "investigate");
        let merged = merge(&provider, &config);
        assert_eq!(merged.labels["team"], "infra");
        assert_eq!(merged.labels[LABEL_RUN_ID], "run-1");
        assert_eq!(merged.labels[LABEL_STEP], "investigate");
    }

    #[test]
    fn sandboxed_refuses_inline_oauth_credentials() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1").oauth_credentials("{}");
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains("refuses inline oauth_credentials"), "{err}");
    }

    #[test]
    fn sandboxed_refuses_plain_api_key() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1").env("ANTHROPIC_API_KEY", "sk");
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains("ANTHROPIC_API_KEY"), "{err}");
        assert!(err.contains("env_from_secret"), "{err}");
    }

    #[test]
    fn sandboxed_refuses_plain_oauth_token() {
        let provider =
            K8sEphemeralProvider::sandboxed("img:v1").env("CLAUDE_CODE_OAUTH_TOKEN", "tok");
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains("CLAUDE_CODE_OAUTH_TOKEN"), "{err}");
    }

    #[test]
    fn non_sandboxed_keeps_inline_credentials() {
        let provider = K8sEphemeralProvider::new("img:v1")
            .oauth_credentials("{}")
            .env("ANTHROPIC_API_KEY", "sk");
        assert!(provider.merged_pod_inputs(&AgentConfig::new("hi")).is_ok());
    }

    #[test]
    fn effective_deadline_explicit_value_wins() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .timeout(Duration::from_secs(600))
            .active_deadline_seconds(Duration::from_secs(30));
        assert_eq!(provider.effective_deadline(), Some(Duration::from_secs(30)));
    }

    #[test]
    fn effective_deadline_sandboxed_is_timeout_plus_margin() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1").timeout(Duration::from_secs(600));
        assert_eq!(
            provider.effective_deadline(),
            Some(Duration::from_secs(660))
        );
    }

    #[test]
    fn effective_deadline_non_sandboxed_defaults_to_none() {
        let provider = K8sEphemeralProvider::new("img:v1").timeout(Duration::from_secs(600));
        assert_eq!(provider.effective_deadline(), None);
    }

    #[test]
    fn deadline_margin_defaults_to_sixty_seconds_when_not_sandboxed() {
        let provider = K8sEphemeralProvider::new("img:v1");
        assert_eq!(
            provider.deadline_margin_or_default(),
            DEFAULT_DEADLINE_MARGIN
        );
    }

    #[test]
    #[should_panic(expected = "orphan reaper interval must be greater than zero")]
    fn spawn_orphan_reaper_zero_interval_panics() {
        drop(K8sEphemeralProvider::sandboxed("img:v1").spawn_orphan_reaper(Duration::ZERO));
    }

    // ── Auth proxy ──────────────────────────────────────────────────

    const PROXY_URL: &str = "http://ironflow-auth-proxy.ironflow-system";

    fn proxied() -> K8sEphemeralProvider {
        K8sEphemeralProvider::sandboxed("img:v1").auth_proxy(PROXY_URL)
    }

    /// The env entries of the agent container, as built for a real pod.
    fn pod_env(
        provider: &K8sEphemeralProvider,
        config: &AgentConfig,
        token: Option<&str>,
    ) -> Vec<Value> {
        let merged = merge(provider, config);
        let env_vars = provider.pod_env_vars(token, &[]);
        let pod = build_pod_spec(&PodConfig {
            name: "claude-code-test",
            image: &provider.image,
            command: vec!["sh".to_string()],
            namespace: &provider.namespace,
            resources: &provider.resources,
            service_account: merged.service_account.as_deref(),
            restart_policy: "Never",
            image_pull_policy: &provider.image_pull_policy,
            env_vars: &env_vars,
            image_pull_secrets: &provider.image_pull_secrets,
            extra_labels: &merged.labels,
            node_selector: &provider.node_selector,
            tolerations: &provider.tolerations,
            volumes: &provider.volumes,
            pvc_volumes: &provider.pvc_volumes,
            inputs: &config.inputs,
            input_init_image: DEFAULT_INPUT_INIT_IMAGE,
            prompt_configmap: None,
            prompt_mount_path: PROMPT_MOUNT_PATH,
            hardening: PodHardening {
                sandbox: provider.sandbox.as_ref(),
                secret_env: &merged.secret_env,
                read_only_volumes: &merged.read_only_volumes,
                step_pvc_volumes: &config.pod.pvc_volumes,
                managed_settings_configmap: None,
                claude_profiles: &provider.claude_profiles,
                sessions_claim: provider.sessions_claim.as_deref(),
                annotations: None,
            },
        })
        .unwrap();
        let pod = to_value(&pod).unwrap();
        pod["spec"]["containers"][0]["env"]
            .as_array()
            .cloned()
            .unwrap_or_default()
    }

    fn env_value<'a>(env: &'a [Value], name: &str) -> Option<&'a str> {
        env.iter()
            .find(|entry| entry["name"] == name)
            .and_then(|entry| entry["value"].as_str())
    }

    #[test]
    fn auth_proxy_builder_sets_url_and_trims_slash() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .auth_proxy("http://ironflow-auth-proxy.ironflow-system/")
            .auth_proxy_admin_key("0123456789abcdef0123456789abcdef");
        assert_eq!(provider.auth_proxy_url.as_deref(), Some(PROXY_URL));
        assert_eq!(
            provider.auth_proxy_admin_key.as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
        assert!(provider.auth_proxy_client().unwrap().is_some());

        let https = K8sEphemeralProvider::new("img:v1").auth_proxy("https://proxy");
        assert_eq!(https.auth_proxy_url.as_deref(), Some("https://proxy"));
    }

    #[test]
    #[should_panic(expected = "auth_proxy url must start with http:// or https://")]
    fn auth_proxy_invalid_url_panics() {
        let _ = K8sEphemeralProvider::sandboxed("img:v1").auth_proxy("ironflow-auth-proxy:80");
    }

    #[test]
    fn auth_proxy_client_is_none_without_proxy() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1").auth_proxy_admin_key("k");
        assert!(provider.auth_proxy_client().unwrap().is_none());
    }

    #[test]
    fn auth_proxy_rejects_oauth_token_from_secret() {
        let provider = proxied().oauth_token_from_secret("claude-oauth", "token");
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains("auth_proxy is set"), "{err}");
        assert!(err.contains("CLAUDE_CODE_OAUTH_TOKEN"), "{err}");
    }

    #[test]
    fn auth_proxy_rejects_step_secret_env_oauth_token() {
        let config = AgentConfig::new("hi").env_from_secret("CLAUDE_CODE_OAUTH_TOKEN", "o", "k");
        let err = err_text(proxied().merged_pod_inputs(&config));
        assert!(err.contains("CLAUDE_CODE_OAUTH_TOKEN"), "{err}");

        let config = AgentConfig::new("hi").env_from_secret("ANTHROPIC_API_KEY", "anthropic", "k");
        let err = err_text(proxied().merged_pod_inputs(&config));
        assert!(err.contains("ANTHROPIC_API_KEY"), "{err}");
    }

    #[test]
    fn auth_proxy_rejects_inline_oauth_credentials() {
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x"}}"#;
        let provider = proxied().oauth_credentials(json);
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains("auth_proxy is set"), "{err}");
        assert!(!err.contains("sk-ant"), "{err}");

        let provider = proxied().oauth_credentials_from_secret("claude-credentials", "creds");
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains(CREDENTIALS_ENV_VAR), "{err}");

        // Not sandboxed: still refused.
        let provider = K8sEphemeralProvider::new("img:v1")
            .auth_proxy(PROXY_URL)
            .oauth_credentials("{}");
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains("auth_proxy is set"), "{err}");
    }

    #[test]
    fn auth_proxy_rejects_sk_ant_plain_value() {
        let provider = K8sEphemeralProvider::new("img:v1")
            .auth_proxy(PROXY_URL)
            .env("SOME_VAR", "sk-ant-oat01-leak");
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains("SOME_VAR"), "{err}");
        assert!(!err.contains("sk-ant-oat01-leak"), "{err}");

        for name in ["ANTHROPIC_BASE_URL", "ANTHROPIC_AUTH_TOKEN"] {
            let provider = proxied().env(name, "x");
            let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
            assert!(err.contains(name), "{err}");
        }
    }

    #[test]
    fn auth_proxy_pod_env_has_base_url_and_opaque_token_only() {
        let provider = proxied().env("TEAM", "infra");
        let config = AgentConfig::new("hi").run_scope("run-1", "review");
        let env = pod_env(&provider, &config, Some("ifap_x"));

        assert_eq!(env_value(&env, "ANTHROPIC_BASE_URL"), Some(PROXY_URL));
        assert_eq!(env_value(&env, "ANTHROPIC_AUTH_TOKEN"), Some("ifap_x"));
        assert_eq!(
            env_value(&env, "CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"),
            Some("1")
        );
        assert_eq!(env_value(&env, "TEAM"), Some("infra"));
        assert!(
            env.iter().all(|entry| entry.get("valueFrom").is_none()),
            "no secretKeyRef expected: {env:?}"
        );
        for name in ["CLAUDE_CODE_OAUTH_TOKEN", "ANTHROPIC_API_KEY"] {
            assert!(
                env_value(&env, name).is_none_or(str::is_empty),
                "{name} must not carry a value: {env:?}"
            );
        }
        let rendered = Value::Array(env).to_string();
        assert!(!rendered.contains("sk-ant"), "{rendered}");
    }

    fn secret_for(env: &str, injection: SecretInjection, hosts: &[&str]) -> ProxiedSecret {
        ProxiedSecret {
            name: env.to_string(),
            env: env.to_string(),
            value: "ghp_real".to_string(),
            injection,
            hosts: hosts.iter().map(|host| host.to_string()).collect(),
        }
    }

    fn oauth2() -> SecretInjection {
        SecretInjection::Basic {
            username: "oauth2".to_string(),
        }
    }

    fn issued(token: &str) -> IssuedToken {
        IssuedToken {
            id: format!("{token}-id"),
            token: token.to_string(),
        }
    }

    fn var_of<'a>(env: &'a [(String, String)], name: &str) -> Option<&'a str> {
        env.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    #[test]
    fn pod_env_vars_sets_secret_token_and_url() {
        let secret = secret_for("GITHUB_TOKEN", SecretInjection::Bearer, &["github.com"]);
        let pairs = [(secret, issued("ifap_gh"))];
        let env = proxied().pod_env_vars(Some("ifap_claude"), &pairs);
        let relay = format!("{PROXY_URL}{RELAY_PREFIX}");
        assert_eq!(var_of(&env, "GITHUB_TOKEN"), Some("ifap_gh"));
        assert_eq!(var_of(&env, "GITHUB_TOKEN_URL"), Some(relay.as_str()));
        assert_eq!(var_of(&env, POD_TOKEN_ENV), Some("ifap_claude"));
        assert_eq!(var_of(&env, GIT_CONFIG_COUNT), None);
        let leaked = env.iter().any(|(_, value)| value.contains("ghp_real"));
        assert!(!leaked, "{env:?}");
    }

    #[test]
    fn pod_env_vars_basic_secret_sets_git_config() {
        let secret = secret_for("GITLAB_TOKEN", oauth2(), &["gitlab.com"]);
        let pairs = [(secret, issued("ifap_gl"))];
        let env = proxied().pod_env_vars(None, &pairs);
        let base = format!("{PROXY_URL}{RELAY_PREFIX}/gitlab.com/");
        let key_0 = format!("url.{base}.insteadOf");
        let key_1 = format!("http.{base}.extraHeader");
        let origin = "https://gitlab.com/";
        assert_eq!(var_of(&env, "GITLAB_TOKEN"), Some("ifap_gl"));
        assert_eq!(var_of(&env, GIT_CONFIG_COUNT), Some("2"));
        assert_eq!(var_of(&env, "GIT_CONFIG_KEY_0"), Some(key_0.as_str()));
        assert_eq!(var_of(&env, "GIT_CONFIG_VALUE_0"), Some(origin));
        assert_eq!(var_of(&env, "GIT_CONFIG_KEY_1"), Some(key_1.as_str()));
        let header = var_of(&env, "GIT_CONFIG_VALUE_1").unwrap();
        let encoded = header.strip_prefix("Authorization: Basic ").unwrap();
        assert_eq!(STANDARD.decode(encoded).unwrap(), b"oauth2:ifap_gl");
        let leaked = env.iter().any(|(_, value)| value.contains("ghp_real"));
        assert!(!leaked, "{env:?}");
    }

    #[test]
    fn pod_env_vars_basic_wildcard_host_has_no_git_rewrite() {
        let secret = secret_for("GITLAB_TOKEN", oauth2(), &["*.gitlab.com"]);
        let pairs = [(secret, issued("ifap_gl"))];
        let env = proxied().pod_env_vars(None, &pairs);
        assert_eq!(var_of(&env, "GITLAB_TOKEN"), Some("ifap_gl"));
        assert_eq!(var_of(&env, GIT_CONFIG_COUNT), None);
    }

    #[test]
    fn merged_proxied_secrets_step_overrides_provider() {
        let github = secret_for("GITHUB_TOKEN", SecretInjection::Bearer, &["github.com"]);
        let npm = secret_for("NPM_TOKEN", SecretInjection::Bearer, &["npmjs.org"]);
        let provider = proxied().proxied_secret(github).proxied_secret(npm);
        let step = secret_for("GITHUB_TOKEN", SecretInjection::Bearer, &["ghe.io"]);
        let config = AgentConfig::new("hi").proxied_secret(step);
        let merged = provider.merged_proxied_secrets(&config);
        let envs: Vec<&str> = merged.iter().map(|secret| secret.env.as_str()).collect();
        assert_eq!(envs, ["GITHUB_TOKEN", "NPM_TOKEN"]);
        assert_eq!(merged[0].hosts, ["ghe.io"]);
        assert_eq!(merge(&provider, &config).proxied_secrets.len(), 2);
    }

    #[test]
    fn proxied_secret_without_auth_proxy_is_an_error() {
        let secret = secret_for("GITHUB_TOKEN", SecretInjection::Bearer, &["github.com"]);
        let config = AgentConfig::new("hi").proxied_secret(secret);
        let provider = K8sEphemeralProvider::sandboxed("img:v1");
        let err = err_text(provider.merged_pod_inputs(&config));
        assert!(err.contains("proxied_secret requires auth_proxy"), "{err}");
        assert!(!err.contains("ghp_real"), "{err}");
    }

    #[test]
    fn proxied_secret_env_collision_is_an_error() {
        let secret = secret_for("GITHUB_TOKEN", SecretInjection::Bearer, &["github.com"]);
        let provider = proxied()
            .env("GITHUB_TOKEN", "x")
            .proxied_secret(secret.clone());
        let err = err_text(provider.merged_pod_inputs(&AgentConfig::new("hi")));
        assert!(err.contains("GITHUB_TOKEN is already set"), "{err}");

        let claude = ProxiedSecret {
            env: "ANTHROPIC_AUTH_TOKEN".to_string(),
            ..secret.clone()
        };
        let config = AgentConfig::new("hi").proxied_secret(claude);
        let err = err_text(proxied().merged_pod_inputs(&config));
        assert!(err.contains("ANTHROPIC_AUTH_TOKEN is already set"), "{err}");

        let other = ProxiedSecret {
            name: "OTHER".to_string(),
            env: "GITHUB_TOKEN_URL".to_string(),
            ..secret.clone()
        };
        let config = AgentConfig::new("hi").proxied_secret(other);
        let provider = proxied().proxied_secret(secret.clone());
        let err = err_text(provider.merged_pod_inputs(&config));
        assert!(err.contains("GITHUB_TOKEN_URL is already set"), "{err}");

        let basic = ProxiedSecret {
            injection: oauth2(),
            ..secret
        };
        let config = AgentConfig::new("hi")
            .env_from_secret("GIT_CONFIG_COUNT", "git", "count")
            .proxied_secret(basic);
        let err = err_text(proxied().merged_pod_inputs(&config));
        assert!(err.contains("must not receive GIT_CONFIG_COUNT"), "{err}");
        assert!(!err.contains("ghp_real"), "{err}");
    }

    #[test]
    #[should_panic(expected = "invalid proxied secret")]
    fn proxied_secret_builder_panics_on_invalid_host() {
        let secret = secret_for("GITHUB_TOKEN", SecretInjection::Bearer, &[".*github.com"]);
        let _ = proxied().proxied_secret(secret);
    }

    #[test]
    fn auth_proxy_account_kind_is_subscription_only_with_proxy() {
        assert_eq!(proxied().account_kind(), Some(ClaudeSubscriptionKind::ID));
        assert_eq!(
            K8sEphemeralProvider::sandboxed("img:v1").account_kind(),
            None
        );
        assert_eq!(K8sEphemeralProvider::new("img:v1").account_kind(), None);
    }

    #[test]
    fn non_proxy_pod_env_unchanged() {
        let provider = K8sEphemeralProvider::sandboxed("img:v1")
            .env("TEAM", "infra")
            .oauth_token_from_secret("claude-oauth", "token");
        assert_eq!(
            provider.pod_env_vars(Some("ifap_x"), &[]),
            vec![("TEAM".to_string(), "infra".to_string())]
        );
        assert_eq!(proxied().pod_env_vars(None, &[]), Vec::new());

        let env = pod_env(&provider, &AgentConfig::new("hi"), None);
        assert!(env_value(&env, "ANTHROPIC_BASE_URL").is_none());
        assert!(env_value(&env, "ANTHROPIC_AUTH_TOKEN").is_none());
        let token = env
            .iter()
            .find(|entry| entry["name"] == "CLAUDE_CODE_OAUTH_TOKEN")
            .expect("token entry");
        assert_eq!(
            token["valueFrom"]["secretKeyRef"]["name"],
            Value::from("claude-oauth")
        );
    }

    // --- Persistent environment ---

    #[test]
    fn ephemeral_provider_environment_volume_builder() {
        let provider = K8sEphemeralProvider::new("img:v1").environment_volume(
            EnvironmentVolume::new("/workspace").size(VolumeSize::new(20, StorageUnit::Gi)),
        );
        let volume = provider.environment.as_ref().unwrap();
        assert_eq!(volume.mount_path, "/workspace");
        assert_eq!(volume.size.to_quantity(), "20Gi");
        assert!(K8sEphemeralProvider::new("img:v1").environment.is_none());
    }

    #[test]
    #[should_panic(expected = "invalid environment volume")]
    fn ephemeral_provider_environment_volume_rejects_relative_path() {
        let _ = K8sEphemeralProvider::new("img:v1")
            .environment_volume(EnvironmentVolume::new("workspace"));
    }

    #[test]
    fn environment_claim_for_new_and_resumed() {
        let fresh = environment_claim_for(None).unwrap();
        assert!(fresh.created);
        assert!(fresh.name.starts_with("ironflow-env-"), "{}", fresh.name);
        assert!(validate_environment_id(&fresh.name).is_ok());
        assert_ne!(fresh.name, environment_claim_for(None).unwrap().name);

        let resumed = environment_claim_for(Some("ironflow-env-abc")).unwrap();
        assert_eq!(
            resumed,
            EnvironmentClaim {
                name: "ironflow-env-abc".to_string(),
                created: false,
            }
        );
    }

    #[test]
    fn environment_claim_for_rejects_invalid_id() {
        let err = environment_claim_for(Some("../kube-system/claim")).unwrap_err();
        assert!(err.to_string().contains("environment_id must only contain"));
        assert!(environment_claim_for(Some("")).is_err());
    }

    #[test]
    fn environment_mount_is_read_write_root_of_claim() {
        let volume = EnvironmentVolume::new("/workspace");
        assert_eq!(
            environment_mount(&volume, "ironflow-env-1"),
            PvcVolume {
                claim_name: "ironflow-env-1".to_string(),
                mount_path: "/workspace".to_string(),
                sub_path: None,
                read_only: false,
            }
        );
    }

    #[test]
    fn build_environment_claim_sets_spec_labels_and_expiry() {
        let volume = EnvironmentVolume::new("/workspace")
            .size(VolumeSize::new(20, StorageUnit::Gi))
            .storage_class("fast");
        let mut pod_labels = BTreeMap::new();
        pod_labels.insert(LABEL_RUN_ID.to_string(), "run-1".to_string());
        pod_labels.insert(LABEL_STEP.to_string(), "build".to_string());
        pod_labels.insert("team".to_string(), "a".to_string());
        let pvc =
            build_environment_claim(&volume, "ironflow-env-1", "ci", &pod_labels, 42).unwrap();
        let value = to_value(&pvc).unwrap();

        assert_eq!(value["metadata"]["name"], "ironflow-env-1");
        assert_eq!(value["metadata"]["namespace"], "ci");
        let labels = &value["metadata"]["labels"];
        assert_eq!(labels[LABEL_MANAGED_BY], MANAGED_BY_IRONFLOW);
        assert_eq!(labels[LABEL_COMPONENT], COMPONENT_ENVIRONMENT);
        assert_eq!(labels[LABEL_RUN_ID], "run-1");
        assert_eq!(labels[LABEL_STEP], "build");
        assert!(labels.get("team").is_none());
        assert_eq!(value["metadata"]["annotations"][LABEL_EXPIRES_AT], "42");
        assert_eq!(value["spec"]["accessModes"], json!(["ReadWriteOnce"]));
        assert_eq!(value["spec"]["resources"]["requests"]["storage"], "20Gi");
        assert_eq!(value["spec"]["storageClassName"], "fast");
        assert!(is_live_environment_claim(&pvc));
    }

    #[test]
    fn build_environment_claim_without_class_uses_cluster_default() {
        let volume = EnvironmentVolume::new("/workspace");
        let pvc =
            build_environment_claim(&volume, "ironflow-env-1", "ci", &BTreeMap::new(), 1).unwrap();
        let value = to_value(&pvc).unwrap();
        assert!(value["spec"].get("storageClassName").is_none());
        assert_eq!(value["spec"]["resources"]["requests"]["storage"], "10Gi");
    }

    #[test]
    fn is_live_environment_claim_rejects_foreign_and_deleting_claims() {
        let foreign: PersistentVolumeClaim = from_value(json!({
            "metadata": { "name": "repos", "labels": { LABEL_MANAGED_BY: "helm" } }
        }))
        .unwrap();
        assert!(!is_live_environment_claim(&foreign));

        let other_component: PersistentVolumeClaim = from_value(json!({
            "metadata": {
                "name": "x",
                "labels": { LABEL_MANAGED_BY: MANAGED_BY_IRONFLOW, LABEL_COMPONENT: "prompt-data" }
            }
        }))
        .unwrap();
        assert!(!is_live_environment_claim(&other_component));

        let deleting: PersistentVolumeClaim = from_value(json!({
            "metadata": {
                "name": "ironflow-env-1",
                "deletionTimestamp": "2023-11-14T22:13:20Z",
                "labels": {
                    LABEL_MANAGED_BY: MANAGED_BY_IRONFLOW,
                    LABEL_COMPONENT: COMPONENT_ENVIRONMENT
                }
            }
        }))
        .unwrap();
        assert!(!is_live_environment_claim(&deleting));
        assert!(!is_live_environment_claim(&PersistentVolumeClaim::default()));
    }

    #[test]
    fn merged_resume_environment_without_volume_is_an_error() {
        let provider = K8sEphemeralProvider::new("img:v1");
        let config = AgentConfig::new("hi").resume_environment("ironflow-env-1");
        let err = err_text(provider.merged_pod_inputs(&config));
        assert!(
            err.contains("resume_environment needs K8sEphemeralProvider::environment_volume"),
            "{err}"
        );
    }

    #[test]
    fn merged_resume_environment_with_volume_is_accepted() {
        let provider = K8sEphemeralProvider::new("img:v1")
            .environment_volume(EnvironmentVolume::new("/workspace"));
        let config = AgentConfig::new("hi").resume_environment("ironflow-env-1");
        assert!(provider.merged_pod_inputs(&config).is_ok());
    }

    #[test]
    fn with_environment_sets_output_environment_id() {
        let output = with_environment(AgentOutput::new(json!("ok")), Some("ironflow-env-1"));
        assert_eq!(output.environment_id.as_deref(), Some("ironflow-env-1"));
        let output = with_environment(AgentOutput::new(json!("ok")), None);
        assert_eq!(output.environment_id, None);
    }

    #[test]
    fn pod_spec_mounts_environment_claim_read_write() {
        let volume = EnvironmentVolume::new("/workspace");
        let step_pvcs = [environment_mount(&volume, "ironflow-env-1")];
        let pod = to_value(
            build_pod_spec(&PodConfig {
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
                hardening: PodHardening {
                    step_pvc_volumes: &step_pvcs,
                    ..PodHardening::default()
                },
            })
            .unwrap(),
        )
        .unwrap();
        let claims: Vec<&str> = pod["spec"]["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v["persistentVolumeClaim"]["claimName"].as_str())
            .collect();
        assert_eq!(claims, vec!["ironflow-env-1"]);
        let mounts = pod["spec"]["containers"][0]["volumeMounts"]
            .as_array()
            .unwrap();
        let mount = mounts
            .iter()
            .find(|m| m["mountPath"] == "/workspace")
            .unwrap();
        assert_ne!(mount["readOnly"], json!(true));
    }
}
