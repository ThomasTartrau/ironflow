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

use futures_util::{AsyncBufReadExt, TryStreamExt};
use k8s_openapi::api::core::v1::{ConfigMap, Pod};
use kube::Client;
use kube::api::{Api, DeleteParams, LogParams, PostParams};
use kube::runtime::wait::await_condition;
use serde_json::{from_value, json};
use tokio::spawn;
use tokio::task::JoinHandle;
use tokio::time;

use tracing::{debug, info, warn};

use crate::account::ClaudeSubscriptionKind;
use crate::auth_proxy::{
    ADMIN_KEY_ENV, AuthProxyClient, AuthProxyError, IssuedToken, POD_BASE_URL_ENV, POD_TOKEN_ENV,
    TokenRequest, resolve_credential,
};
use crate::error::AgentError;
use crate::provider::{
    AgentConfig, AgentOutput, AgentProvider, InvokeFuture, LABEL_EGRESS_PROFILE, LABEL_EXPIRES_AT,
    LABEL_ROOT_RUN_ID, LABEL_RUN_ID, LABEL_STEP, LogSink, PodVolumeSource, ReadOnlyVolume,
    ReleaseFuture, SecretEnvVar, assert_pod_label_allowed, is_reserved_pod_label,
    upsert_secret_env,
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

/// Label selector matching every pod and Job created by ironflow: agent pods,
/// `PodRun` and `JobRun` of `ironflow-ops-k8s`.
pub(super) const MANAGED_SELECTOR: &str = "app.kubernetes.io/managed-by=ironflow";

/// Label selector matching every agent pod created by ironflow.
pub(super) const RUNNER_SELECTOR: &str =
    "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=claude-runner";

/// Label selector matching every prompt ConfigMap created by ironflow.
pub(super) const PROMPT_SELECTOR: &str =
    "app.kubernetes.io/managed-by=ironflow,app.kubernetes.io/component=prompt-data";

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
    auth_proxy_url: Option<String>,
    auth_proxy_admin_key: Option<String>,
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
            auth_proxy_url: None,
            auth_proxy_admin_key: None,
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
    /// Id of the auth proxy token issued for the pod, revoked at the end of the step.
    proxy_token_id: Option<String>,
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
}

impl K8sEphemeralProvider {
    /// Merge the provider settings with the step's, the step winning.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ProcessFailed`] when a sandboxed provider carries
    /// a secret as plain text, when the auth proxy is set and the pod would
    /// receive a Claude credential, or when the managed-settings preset is unknown.
    fn merged_pod_inputs<'a>(
        &'a self,
        config: &AgentConfig,
    ) -> Result<MergedPodInputs<'a>, AgentError> {
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

        let service_account = config
            .pod
            .service_account
            .clone()
            .or_else(|| self.service_account.clone());

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

    /// Plain environment variables of the pod: the provider's, plus the proxy
    /// URL and the opaque token when the auth proxy is set and a token was issued.
    fn pod_env_vars(&self, proxy_token: Option<&str>) -> Vec<(String, String)> {
        let mut env = self.env_vars.clone();
        if let (Some(url), Some(token)) = (&self.auth_proxy_url, proxy_token) {
            env.push((POD_BASE_URL_ENV.to_string(), url.clone()));
            env.push((POD_TOKEN_ENV.to_string(), token.to_string()));
            env.push((NONESSENTIAL_TRAFFIC_ENV.to_string(), "1".to_string()));
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

    /// Revoke the opaque token of a step, best effort: its expiry is the backstop.
    async fn revoke_proxy_token(&self, id: Option<&str>) {
        let Some(id) = id else {
            return;
        };
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

    /// Delete the orphaned ironflow pods, Jobs and prompt ConfigMaps of the
    /// provider's namespace: [`reap_orphans`] with the provider's cluster and
    /// namespace.
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
        let env_vars = self.pod_env_vars(issued.as_ref().map(|t| t.token.as_str()));
        let proxy_token_id = issued.map(|t| t.id);

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
                step_pvc_volumes: &config.pod.pvc_volumes,
                managed_settings_configmap: merged.managed_settings_configmap.as_deref(),
                claude_profiles: &self.claude_profiles,
                annotations: Some(&annotations),
            },
        });

        let created = match pod_spec {
            Ok(mut pod_spec) => {
                // Applied after build_pod_spec: the shared PodConfig builder does not
                // carry this field, so it is set on the built pod here.
                apply_active_deadline_seconds(&mut pod_spec, self.effective_deadline());
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
            self.revoke_proxy_token(proxy_token_id.as_deref()).await;
            abort_launch_configmap(&configmaps, prompt_configmap_name.as_deref()).await;
            return Err(e);
        }

        Ok(CreatedPod {
            pods,
            pod_name,
            start,
            prompt_configmap: prompt_configmap_name,
            configmaps: Some(configmaps),
            proxy_token_id,
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
        let request = TokenRequest {
            run_id: run_id.map_or_else(|| pod_name.to_string(), String::clone),
            step: step.map_or_else(|| "agent".to_string(), String::clone),
            expires_at,
            credential,
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

        self.finalize_pod(&logs, &pod_phase, timed_out, pod_name, config, *start)
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

        self.finalize_pod(
            &accumulated,
            &pod_phase,
            timed_out,
            pod_name,
            config,
            *start,
        )
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
            self.revoke_proxy_token(created.proxy_token_id.as_deref())
                .await;
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
            self.revoke_proxy_token(created.proxy_token_id.as_deref())
                .await;
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
        let env_vars = provider.pod_env_vars(token);
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
            provider.pod_env_vars(Some("ifap_x")),
            vec![("TEAM".to_string(), "infra".to_string())]
        );
        assert_eq!(proxied().pod_env_vars(None), Vec::new());

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
}
