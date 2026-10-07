//! Provider trait and configuration types for agent invocations.
//!
//! The [`AgentProvider`] trait is the primary extension point in ironflow: implement it
//! to plug in any AI backend (local model, HTTP API, mock, etc.) without changing
//! your workflow code.
//!
//! Built-in implementations:
//!
//! * [`ClaudeCodeProvider`](crate::providers::claude::ClaudeCodeProvider) - local `claude` CLI.
//! * `SshProvider` - remote via SSH (requires `transport-ssh` feature).
//! * `DockerProvider` - Docker container (requires `transport-docker` feature).
//! * `K8sEphemeralProvider` - ephemeral K8s pod (requires `transport-k8s` feature).
//! * `K8sPersistentProvider` - persistent K8s pod (requires `transport-k8s` feature).
//! * [`RecordReplayProvider`](crate::providers::record_replay::RecordReplayProvider) -
//!   records and replays fixtures for deterministic testing.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::account::{AccountSession, RateLimitRecorder};
use crate::auth_proxy::ProxiedSecret;
use crate::error::AgentError;
use crate::operations::agent::{Model, PermissionMode};
use crate::retry::RetryPolicy;
use crate::trace_context::WorkflowTraceContext;

mod pod;
mod system_prompt;
mod tool;
mod tool_profile;

#[cfg(feature = "transport-k8s")]
pub(crate) use pod::validate_environment_id;
pub use pod::{
    COMPONENT_ENVIRONMENT, EnvironmentVolume, LABEL_COMPONENT, LABEL_EGRESS_PROFILE,
    LABEL_EXPIRES_AT, LABEL_MANAGED_BY, LABEL_ROOT_RUN_ID, LABEL_RUN_ID, LABEL_STEP,
    MANAGED_BY_IRONFLOW, PodSettings, PodVolumeSource, PvcVolume, ReadOnlyVolume, SecretEnvVar,
    StorageUnit, VolumeSize, assert_pod_label_allowed, is_reserved_pod_label, sanitize_label_value,
    validate_pvc_sub_path,
};
pub(crate) use pod::{assert_environment_id_valid, upsert_proxied_secret, upsert_secret_env};
pub use tool::Tool;
pub use tool_profile::ToolProfile;

/// Boxed future returned by [`AgentProvider::invoke`].
pub type InvokeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AgentOutput, AgentError>> + Send + 'a>>;

/// Boxed future returned by [`AgentProvider::release_run`].
pub type ReleaseFuture<'a> = Pin<Box<dyn Future<Output = Result<(), AgentError>> + Send + 'a>>;

// ── Typestate markers ──────────────────────────────────────────────

/// Marker: no tools have been added via the builder.
#[derive(Debug, Clone, Copy)]
pub struct NoTools;

/// Marker: at least one tool has been added via [`AgentConfig::allow_tool`],
/// or a tool profile selected via [`AgentConfig::tool_profile`].
#[derive(Debug, Clone, Copy)]
pub struct WithTools;

/// Marker: no JSON schema has been set via the builder.
#[derive(Debug, Clone, Copy)]
pub struct NoSchema;

/// Marker: a JSON schema derived from `T` has been set via
/// [`AgentConfig::output`]. The step answers with a `T`.
pub struct WithSchema<T>(PhantomData<fn() -> T>);

// Written by hand: a derive would require `T: Debug + Clone + Copy` for a
// marker that never holds a `T`.
impl<T> fmt::Debug for WithSchema<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WithSchema")
    }
}

impl<T> Clone for WithSchema<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for WithSchema<T> {}

/// Marker: a pre-serialized JSON schema has been set via
/// [`AgentConfig::output_schema_raw`]. The answer is not typed.
#[derive(Debug, Clone, Copy)]
pub struct RawSchema;

// ── AgentInput ─────────────────────────────────────────────────────

/// Declarative external input fetched into the agent's filesystem before invocation.
///
/// Each input is a URL that the provider must download and materialize at
/// `mount_path` so the agent can read it via the `Read` tool.
///
/// Provider behavior:
///
/// * [`ClaudeCodeProvider`](crate::providers::claude::ClaudeCodeProvider) (local) -
///   downloads via reqwest into a per-invocation temp directory and rewrites
///   `mount_path` to the resolved local path.
/// * `K8sEphemeralProvider` - injects a `curlimages/curl` initContainer that
///   downloads each URL into a shared `emptyDir`, mounted on the main container
///   at the parent directory of `mount_path`.
///
/// The `mount_path` must be an absolute path. Intermediate directories are
/// created automatically.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentInput {
    /// Source URL to download (HTTP/HTTPS, including signed S3/R2 URLs).
    pub url: String,

    /// Absolute filesystem path where the file must be available inside the
    /// agent's filesystem.
    pub mount_path: String,
}

impl AgentInput {
    /// Create a new input descriptor.
    pub fn new(url: &str, mount_path: &str) -> Self {
        Self {
            url: url.to_string(),
            mount_path: mount_path.to_string(),
        }
    }
}

// ── AgentConfig ────────────────────────────────────────────────────

/// Serializable configuration passed to an [`AgentProvider`] for a single invocation.
///
/// Built by [`Agent::run`](crate::operations::agent::Agent::run) from the builder state.
/// Provider implementations translate these fields into whatever format the underlying
/// backend expects.
///
/// # Typestate: tools vs structured output
///
/// Claude CLI has a [known bug](https://github.com/anthropics/claude-code/issues/18536)
/// where combining `--json-schema` with `--allowedTools` always returns
/// `structured_output: null`. To prevent this at compile time, [`allow_tool`](Self::allow_tool)
/// and [`output`](Self::output) / [`output_schema_raw`](Self::output_schema_raw) are mutually
/// exclusive: using one removes the other from the available API.
///
/// ```
/// use ironflow_core::provider::{AgentConfig, Tool};
///
/// // OK: tools only
/// let _ = AgentConfig::new("search").allow_tool(Tool::WebSearch);
///
/// // OK: structured output only
/// let _ = AgentConfig::new("classify").output_schema_raw(r#"{"type":"object"}"#);
/// ```
///
/// ```compile_fail,E0599
/// use ironflow_core::provider::{AgentConfig, Tool};
/// // COMPILE ERROR: cannot add tools after setting structured output
/// let _ = AgentConfig::new("x").output_schema_raw("{}").allow_tool(Tool::Read);
/// ```
///
/// ```compile_fail,E0599
/// use ironflow_core::provider::{AgentConfig, Tool};
/// // COMPILE ERROR: cannot set structured output after adding tools
/// let _ = AgentConfig::new("x").allow_tool(Tool::Read).output_schema_raw("{}");
/// ```
///
/// ```compile_fail,E0599
/// use ironflow_core::provider::{AgentConfig, ToolProfile};
/// // COMPILE ERROR: a tool profile counts as tools
/// let _ = AgentConfig::new("x").tool_profile(ToolProfile::new("bug")).output_schema_raw("{}");
/// ```
///
/// **Workaround**: split the work into two steps -- one agent with tools to
/// gather data, then a second agent with `.output::<T>()` to structure the result.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(bound(serialize = "", deserialize = ""))]
#[non_exhaustive]
pub struct AgentConfig<Tools = NoTools, Schema = NoSchema> {
    /// Optional system prompt that sets the agent's persona or constraints.
    pub system_prompt: Option<String>,

    /// Optional text appended to the system prompt instead of replacing it.
    ///
    /// On the Claude CLI this is `--append-system-prompt`: Claude Code keeps
    /// its own system prompt (skills, slash commands, tools) and adds this
    /// text after it. HTTP providers append it to
    /// [`system_prompt`](Self::system_prompt), separated by a blank line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub append_system_prompt: Option<String>,

    /// The user prompt - the main instruction to the agent.
    pub prompt: String,

    /// Which model to use for this invocation.
    ///
    /// Accepts any string. Use [`Model`] constants for well-known Claude models
    /// (e.g. `Model::SONNET`), or pass a custom identifier for other providers.
    #[serde(default = "default_model")]
    pub model: String,

    /// Allowlist of tool names the agent may invoke (empty = provider default).
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Denylist of tool names the agent MUST NOT invoke.
    ///
    /// Maps to `--disallowedTools` on the Claude CLI. Unlike
    /// [`allowed_tools`](Self::allowed_tools), this does **not** activate any
    /// tools; it only filters out tools that would otherwise be loaded by
    /// default. As such, it is safe to combine with structured output
    /// ([`output`](Self::output)) without triggering the Claude CLI bug that
    /// affects `--json-schema` + `--allowedTools`.
    #[serde(default)]
    pub disallowed_tools: Vec<String>,

    /// Named tool profile the provider exposes to this step.
    ///
    /// Set it with [`AgentConfig::tool_profile`]. `None` means the provider's
    /// default tools only (none unless it has some).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_profile: Option<ToolProfile>,

    /// Maximum number of agentic turns before the provider should stop.
    pub max_turns: Option<u32>,

    /// Maximum number of tool calls executed concurrently within a single
    /// turn's group of consecutive read-only calls (default 4). `1` restores
    /// fully sequential tool execution.
    #[serde(default = "default_max_parallel_tools")]
    pub max_parallel_tools: usize,

    /// Maximum spend in USD for this single invocation.
    pub max_budget_usd: Option<f64>,

    /// Working directory for the agent process.
    pub working_dir: Option<String>,

    /// Path to an MCP server configuration file.
    pub mcp_config: Option<String>,

    /// When `true`, pass `--strict-mcp-config` to the Claude CLI so it only
    /// loads MCP servers from [`mcp_config`](Self::mcp_config) and ignores
    /// any global/user MCP configuration (e.g. `~/.claude.json`).
    ///
    /// Useful to prevent global MCP servers from leaking tools into steps
    /// that request `structured_output`, which triggers the Claude CLI bug
    /// where `--json-schema` combined with any active tool returns
    /// `structured_output: null`. See
    /// <https://github.com/anthropics/claude-code/issues/18536>.
    ///
    /// Combine with `mcp_config` set to a file containing
    /// `{"mcpServers":{}}` to disable every MCP server for the invocation.
    #[serde(default)]
    pub strict_mcp_config: bool,

    /// When `true`, pass `--bare` to Claude CLI. Bare mode disables:
    /// - auto-memory (automatic creation of `~/.claude/.../memory/*.md` files)
    /// - `CLAUDE.md` auto-discovery (no global/project `CLAUDE.md` loaded)
    /// - hooks, LSP, plugin sync, attribution, background prefetches
    ///
    /// Recommended for orchestrator agents that should not have any implicit
    /// side effects on the user's filesystem or inherit user-level context.
    ///
    /// # Authentication requirement
    ///
    /// `--bare` is **only compatible with an Anthropic API key**
    /// (`ANTHROPIC_API_KEY` environment variable). It does **not** work with
    /// OAuth authentication (`claude /login` / keychain-stored credentials),
    /// because bare mode disables keychain reads.
    #[serde(default)]
    pub bare: bool,

    /// Permission mode controlling how the agent handles tool-use approvals.
    #[serde(default)]
    pub permission_mode: PermissionMode,

    /// Optional JSON Schema string. When set, the provider should request
    /// structured (typed) output from the model.
    #[serde(alias = "output_schema")]
    pub json_schema: Option<String>,

    /// Optional session ID to resume a previous conversation.
    ///
    /// When set, the provider should continue the conversation from the
    /// specified session rather than starting a new one.
    pub resume_session_id: Option<String>,

    /// Session id to create (`--session-id <uuid>`).
    ///
    /// Fixes the id of the session the provider starts, so it can be resumed
    /// later through [`AgentConfig::resume_session_id`]. Ignored when
    /// `resume_session_id` is set. Providers without sessions ignore it.
    #[serde(default)]
    pub session_id: Option<String>,

    /// Prompt the engine sends instead of `prompt` when it resumes an
    /// interrupted step.
    ///
    /// Read by the engine/executor only; providers ignore it.
    #[serde(default)]
    pub resume_prompt: Option<String>,

    /// Optional persistent environment ID to resume (K8s ephemeral provider only).
    ///
    /// The ID is the name of a PersistentVolumeClaim previously returned in
    /// [`AgentOutput::environment_id`]. When set, the provider mounts that
    /// volume again instead of creating a fresh one, so the files written by
    /// a previous step are still there. Providers without persistent
    /// environments ignore this field.
    #[serde(default)]
    pub resume_environment_id: Option<String>,

    /// Enable verbose/debug mode to capture the full conversation trace.
    ///
    /// When `true`, the provider uses streaming output (`stream-json`) to
    /// record every assistant message and tool call. The resulting
    /// [`AgentOutput::debug_messages`] field will contain the conversation
    /// trace for inspection.
    #[serde(default)]
    pub verbose: bool,

    /// Custom labels applied to the pod (K8s providers only).
    ///
    /// Non-K8s providers ignore this field. Labels are merged with the
    /// provider-level pod labels and the hardcoded ironflow labels. In case
    /// of conflict, hardcoded labels always win, then invocation-level labels,
    /// then provider-level defaults.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pod_labels: BTreeMap<String, String>,

    /// Pod-level settings (K8s ephemeral provider only), merged with the provider's.
    ///
    /// Non-K8s providers ignore this field. Set it with
    /// [`AgentConfig::env_from_secret`], [`AgentConfig::service_account`],
    /// [`AgentConfig::read_only_volume`], [`AgentConfig::managed_settings`] and
    /// [`AgentConfig::runtime_class`].
    #[serde(default, skip_serializing_if = "PodSettings::is_empty")]
    pub pod: PodSettings,

    /// External inputs to materialize on the agent's filesystem before invocation.
    ///
    /// See [`AgentInput`] for the semantics. The provider is responsible for
    /// fetching each URL and placing it at `mount_path` before the agent runs.
    /// Add inputs with [`AgentConfig::input_file`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<AgentInput>,

    /// When `true`, a failure of this step does not fail the run.
    #[serde(default)]
    pub allow_failure: bool,

    /// Optional step-level retry policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,

    /// Optional W3C trace context for distributed tracing propagation.
    ///
    /// When set, providers can inject the `traceparent` header into
    /// outgoing HTTP requests (LLM APIs, MCP servers) to correlate
    /// workflow spans with downstream service spans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<WorkflowTraceContext>,

    /// Provider Account the invocation runs under, set by the worker.
    ///
    /// Providers whose [`AgentProvider::account_kind`] matches inject its
    /// credential into the agent process and report observed rate-limit
    /// windows to its recorder. Never serialized: it carries a secret.
    #[serde(skip)]
    pub account: Option<AccountSession>,

    /// Secrets the agent reaches through the auth proxy (K8s ephemeral
    /// provider only), merged with the provider's: a step entry overrides a
    /// provider entry with the same `env`.
    ///
    /// Set it with [`AgentConfig::proxied_secret`]. Never serialized: it
    /// carries a secret.
    #[serde(skip)]
    pub proxied_secrets: Vec<ProxiedSecret>,

    /// Longest the run may sleep waiting for provider capacity when every
    /// targeted account is rate limited.
    ///
    /// `None` uses the worker default (6 hours). `Duration::ZERO` fails the
    /// step at once with [`AgentError::NoCapacity`]. Set it with
    /// [`AgentConfig::max_capacity_wait`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_capacity_wait: Option<Duration>,

    /// Name of the Provider Account the step must run under.
    ///
    /// Never falls back to another account: an unknown name fails with
    /// [`AgentError::AccountNotFound`], a rate-limited one waits. Set it with
    /// [`AgentConfig::account`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,

    /// Tag restricting the step to the Provider Accounts carrying it.
    ///
    /// Set it with [`AgentConfig::account_pool`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_pool: Option<String>,

    /// Recorder observed rate-limit windows go to when the invocation runs
    /// without an [`AccountSession`] (the worker environment token).
    ///
    /// Set by the worker. Ignored when [`account`](Self::account) is set.
    #[serde(skip)]
    pub rate_limits: Option<RateLimitRecorder>,

    /// When the step started waiting for provider capacity, set by the engine.
    ///
    /// Bounds the cumulative wait for an account freed by `max_concurrency`.
    #[serde(skip)]
    pub capacity_wait_since: Option<DateTime<Utc>>,

    /// Zero-sized typestate marker (not serialized).
    #[serde(skip)]
    pub(crate) _marker: PhantomData<(Tools, Schema)>,
}

fn default_model() -> String {
    Model::SONNET.to_string()
}

fn default_max_parallel_tools() -> usize {
    4
}

// ── Constructor (base type only) ───────────────────────────────────

impl AgentConfig {
    /// Create an `AgentConfig` with required fields and defaults for the rest.
    pub fn new(prompt: &str) -> Self {
        Self {
            system_prompt: None,
            append_system_prompt: None,
            prompt: prompt.to_string(),
            model: Model::SONNET.to_string(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            tool_profile: None,
            max_turns: None,
            max_parallel_tools: 4,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            strict_mcp_config: false,
            bare: false,
            permission_mode: PermissionMode::Default,
            json_schema: None,

            resume_session_id: None,
            session_id: None,
            resume_prompt: None,
            resume_environment_id: None,
            verbose: false,
            pod_labels: BTreeMap::new(),
            pod: PodSettings::default(),
            inputs: Vec::new(),
            allow_failure: false,
            retry: None,
            trace_context: None,
            account: None,
            proxied_secrets: Vec::new(),
            max_capacity_wait: None,
            account_name: None,
            account_pool: None,
            rate_limits: None,
            capacity_wait_since: None,
            _marker: PhantomData,
        }
    }
}

// ── Methods available on ALL typestate variants ────────────────────

impl<Tools, Schema> AgentConfig<Tools, Schema> {
    /// Set the system prompt.
    pub fn system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = Some(prompt.to_string());
        self
    }

    /// Set the model name.
    pub fn model(mut self, model: &str) -> Self {
        self.model = model.to_string();
        self
    }

    /// Set the maximum budget in USD.
    pub fn max_budget_usd(mut self, budget: f64) -> Self {
        self.max_budget_usd = Some(budget);
        self
    }

    /// Set the maximum number of turns.
    pub fn max_turns(mut self, turns: u32) -> Self {
        self.max_turns = Some(turns);
        self
    }

    /// Set the maximum number of tool calls executed concurrently within a
    /// turn's group of consecutive read-only calls.
    ///
    /// # Panics
    ///
    /// Panics if `n` is `0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("summarize the repo").max_parallel_tools(2);
    /// assert_eq!(config.max_parallel_tools, 2);
    /// ```
    pub fn max_parallel_tools(mut self, n: usize) -> Self {
        assert!(n > 0, "max_parallel_tools must be greater than 0");
        self.max_parallel_tools = n;
        self
    }

    /// Set the working directory.
    pub fn working_dir(mut self, dir: &str) -> Self {
        self.working_dir = Some(dir.to_string());
        self
    }

    /// Set the permission mode.
    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.permission_mode = mode;
        self
    }

    /// Enable verbose/debug mode.
    pub fn verbose(mut self, enabled: bool) -> Self {
        self.verbose = enabled;
        self
    }

    /// Set the MCP server configuration file path.
    pub fn mcp_config(mut self, config: &str) -> Self {
        self.mcp_config = Some(config.to_string());
        self
    }

    /// Enable strict MCP config mode.
    ///
    /// When `true`, the Claude CLI is invoked with `--strict-mcp-config`,
    /// which disables loading of any MCP server defined outside the
    /// [`mcp_config`](Self::mcp_config) file (the global `~/.claude.json`
    /// and user-level configs are ignored).
    ///
    /// This is the recommended way to prevent global MCP servers from
    /// silently injecting tools into a structured-output step and
    /// triggering the Claude CLI bug that returns `structured_output: null`
    /// whenever any tool is active. See
    /// <https://github.com/anthropics/claude-code/issues/18536>.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    /// use schemars::JsonSchema;
    ///
    /// #[derive(serde::Deserialize, JsonSchema)]
    /// struct Out { ok: bool }
    ///
    /// // Isolate the step from any global MCP server so structured output works.
    /// let config = AgentConfig::new("classify this")
    ///     .strict_mcp_config(true)
    ///     .mcp_config(r#"{"mcpServers":{}}"#)
    ///     .output::<Out>();
    /// ```
    pub fn strict_mcp_config(mut self, strict: bool) -> Self {
        self.strict_mcp_config = strict;
        self
    }

    /// Enable bare mode (minimal Claude Code environment, see `--bare`).
    ///
    /// When `true`, the Claude CLI is invoked with `--bare`, which disables:
    /// - auto-memory (no automatic `~/.claude/.../memory/*.md` file creation)
    /// - `CLAUDE.md` auto-discovery (neither global nor project-level)
    /// - hooks, LSP, plugin sync, attribution, background prefetches,
    ///   keychain reads
    ///
    /// Sets `CLAUDE_CODE_SIMPLE=1` in the child process.
    ///
    /// Recommended for orchestrator steps that should not have any implicit
    /// side effects on the user's filesystem or inherit user-level context
    /// (email, preferences, etc.).
    ///
    /// # Authentication requirement
    ///
    /// `--bare` is **only compatible with an Anthropic API key**
    /// (`ANTHROPIC_API_KEY` environment variable). It does **not** work with
    /// OAuth authentication (`claude /login` / keychain-stored credentials),
    /// because bare mode disables keychain reads. Invoking a bare agent on an
    /// OAuth-only host will fail with an authentication error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("classify this")
    ///     .bare(true);
    /// ```
    pub fn bare(mut self, enabled: bool) -> Self {
        self.bare = enabled;
        self
    }

    /// Mark this step as allowed to fail without stopping the run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("lint the code").allow_failure();
    /// assert!(config.allow_failure);
    /// ```
    pub fn allow_failure(mut self) -> Self {
        self.allow_failure = true;
        self
    }

    /// Replace the entire disallowed-tools list.
    ///
    /// Maps to `--disallowedTools` on the Claude CLI. This method is available
    /// on **every** typestate variant (including
    /// [`AgentConfig<NoTools, WithSchema<T>>`]) because, unlike
    /// [`allow_tool`](AgentConfig::allow_tool), `disallowed_tools` does not
    /// activate any tool -- it only filters out tools that would otherwise be
    /// loaded by default.
    ///
    /// As such, it is safe to combine with structured output:
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::{AgentConfig, Tool};
    /// use schemars::JsonSchema;
    ///
    /// #[derive(serde::Deserialize, JsonSchema)]
    /// struct Out { ok: bool }
    ///
    /// let config = AgentConfig::new("classify this")
    ///     .disallowed_tools([Tool::Write, Tool::Edit])
    ///     .output::<Out>();
    /// assert_eq!(config.disallowed_tools, vec!["Write", "Edit"]);
    /// ```
    pub fn disallowed_tools<I>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = Tool>,
    {
        self.disallowed_tools = tools.into_iter().map(|tool| tool.to_string()).collect();
        self
    }

    /// Add a single custom pod label (K8s providers only).
    ///
    /// Can be called multiple times. Non-K8s providers ignore this field.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("analyze")
    ///     .pod_label("ironflow.io/network-profile", "grafana-only")
    ///     .pod_label("team", "observability");
    /// ```
    pub fn pod_label(mut self, key: &str, value: &str) -> Self {
        self.pod_labels.insert(key.to_string(), value.to_string());
        self
    }

    /// Replace the entire custom pod labels map (K8s providers only).
    ///
    /// Non-K8s providers ignore this field.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeMap;
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let mut labels = BTreeMap::new();
    /// labels.insert("env".to_string(), "staging".to_string());
    /// let config = AgentConfig::new("deploy").pod_labels(labels);
    /// ```
    pub fn pod_labels(mut self, labels: BTreeMap<String, String>) -> Self {
        self.pod_labels = labels;
        self
    }

    /// Set a session ID to resume a previous conversation.
    pub fn resume(mut self, session_id: &str) -> Self {
        self.resume_session_id = Some(session_id.to_string());
        self
    }

    /// Fix the id of the session the provider creates (`--session-id <uuid>`).
    ///
    /// Ignored when a session to resume is set with [`AgentConfig::resume`].
    /// Claude Code requires a UUID.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review the code")
    ///     .session_id("0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d");
    /// assert!(config.session_id.is_some());
    /// ```
    pub fn session_id(mut self, session_id: &str) -> Self {
        self.session_id = Some(session_id.to_string());
        self
    }

    /// Set the prompt sent when the engine resumes an interrupted step.
    ///
    /// The engine resumes the Claude Code session of an agent step
    /// interrupted by a lost worker lease and sends this prompt instead of
    /// the original one. Without it, the engine uses its default resume
    /// prompt. Providers ignore this field.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("migrate the schema")
    ///     .resume_prompt("You were interrupted. Check the migration state and finish it.");
    /// assert!(config.resume_prompt.is_some());
    /// ```
    pub fn resume_prompt(mut self, prompt: &str) -> Self {
        self.resume_prompt = Some(prompt.to_string());
        self
    }

    /// Resume a persistent environment created by a previous agent step.
    ///
    /// `environment_id` is the value of [`AgentOutput::environment_id`]
    /// (the name of a PersistentVolumeClaim). Only the K8s ephemeral
    /// provider configured with an environment volume honours it and fails
    /// the step when the environment does not exist anymore. Other providers
    /// ignore it.
    ///
    /// # Panics
    ///
    /// Panics if `environment_id` is empty, longer than 253 characters, or
    /// contains characters other than lowercase ASCII letters, digits and
    /// `-` (a DNS-1123 name).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("run the tests again")
    ///     .resume_environment("ironflow-env-0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d");
    /// assert!(config.resume_environment_id.is_some());
    /// ```
    pub fn resume_environment(mut self, environment_id: &str) -> Self {
        assert_environment_id_valid(environment_id);
        self.resume_environment_id = Some(environment_id.to_string());
        self
    }

    /// Set a step-level retry policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    /// use ironflow_core::retry::RetryPolicy;
    ///
    /// let config = AgentConfig::new("Summarize this document")
    ///     .retry_policy(RetryPolicy::new(3));
    /// assert!(config.retry.is_some());
    /// ```
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = Some(policy);
        self
    }

    /// Attach a [`WorkflowTraceContext`] for distributed tracing.
    ///
    /// When set, providers can inject the `traceparent` header into
    /// outgoing HTTP requests to correlate workflow spans with
    /// downstream service spans.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    /// use ironflow_core::trace_context::WorkflowTraceContext;
    ///
    /// let ctx = WorkflowTraceContext::new_root();
    /// let config = AgentConfig::new("classify this")
    ///     .trace_context(ctx);
    /// assert!(config.trace_context.is_some());
    /// ```
    pub fn trace_context(mut self, ctx: WorkflowTraceContext) -> Self {
        self.trace_context = Some(ctx);
        self
    }

    /// Declare an external input that the provider must materialize on the
    /// agent's filesystem before invocation.
    ///
    /// `url` is fetched (HTTP/HTTPS) and written to `mount_path` (absolute
    /// path) inside the agent's runtime. Each provider materializes inputs
    /// in its own way:
    ///
    /// * Local provider: downloads to a temp dir on the host.
    /// * K8s providers: spawn a `curlimages/curl` initContainer that downloads
    ///   into a shared `emptyDir` mounted on the main container.
    ///
    /// Can be called multiple times to declare several inputs.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::{AgentConfig, Tool};
    ///
    /// let config = AgentConfig::new("Read /work/dossier.pdf and summarize")
    ///     .allow_tool(Tool::Read)
    ///     .input_file("https://r2.example.com/dossier.pdf", "/work/dossier.pdf");
    /// ```
    pub fn input_file(mut self, url: &str, mount_path: &str) -> Self {
        self.inputs.push(AgentInput::new(url, mount_path));
        self
    }

    /// Read an environment variable from a Kubernetes Secret (K8s ephemeral
    /// provider only).
    ///
    /// The pod gets `valueFrom.secretKeyRef`, so the value never enters the
    /// pod spec. Calling it again with the same `var` replaces the entry. A
    /// step entry overrides a provider entry with the same name.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("open the MR")
    ///     .env_from_secret("GITLAB_TOKEN", "gitlab-bot", "token");
    /// assert_eq!(config.pod.secret_env[0].secret, "gitlab-bot");
    /// ```
    pub fn env_from_secret(mut self, var: &str, secret: &str, key: &str) -> Self {
        let entry = SecretEnvVar {
            name: var.to_string(),
            secret: secret.to_string(),
            key: key.to_string(),
        };
        upsert_secret_env(&mut self.pod.secret_env, entry);
        self
    }

    /// Hand a secret to the agent through the auth proxy (K8s ephemeral
    /// provider with an auth proxy only).
    ///
    /// The pod gets `<env>` set to an opaque token and `<env>_URL` set to
    /// `<proxy>/r`, never the value: the proxy injects the real secret on
    /// requests to the allowlisted hosts. Calling it again with the same
    /// `env` replaces the entry. A step entry overrides a provider entry
    /// with the same `env`.
    ///
    /// # Panics
    ///
    /// Panics when [`ProxiedSecret::validate`] refuses `secret`: invalid
    /// `env` or name, empty or invalid host allowlist, invalid injection.
    /// The message never carries the value.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::env::{VarError, var};
    ///
    /// use ironflow_core::auth_proxy::{ProxiedSecret, SecretInjection};
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// # fn example() -> Result<(), VarError> {
    /// let config = AgentConfig::new("open the PR").proxied_secret(ProxiedSecret {
    ///     name: "GITHUB_TOKEN".to_string(),
    ///     env: "GITHUB_TOKEN".to_string(),
    ///     value: var("GITHUB_TOKEN")?,
    ///     injection: SecretInjection::Bearer,
    ///     hosts: vec!["api.github.com".to_string()],
    /// });
    /// assert_eq!(config.proxied_secrets[0].env, "GITHUB_TOKEN");
    /// # Ok(())
    /// # }
    /// ```
    pub fn proxied_secret(mut self, secret: ProxiedSecret) -> Self {
        upsert_proxied_secret(&mut self.proxied_secrets, secret);
        self
    }

    /// Run the pod under a given Kubernetes service account (K8s ephemeral
    /// provider only). Overrides the provider's service account.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("read the cluster").service_account("reader");
    /// assert_eq!(config.pod.service_account.as_deref(), Some("reader"));
    /// ```
    pub fn service_account(mut self, name: &str) -> Self {
        self.pod.service_account = Some(name.to_string());
        self
    }

    /// Run the pod under a given Kubernetes RuntimeClass, for instance
    /// `gvisor` (K8s ephemeral provider only). Overrides the provider's.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("x").runtime_class("gvisor");
    /// assert_eq!(config.pod.runtime_class.as_deref(), Some("gvisor"));
    /// ```
    pub fn runtime_class(mut self, name: &str) -> Self {
        self.pod.runtime_class = Some(name.to_string());
        self
    }

    /// Mount a volume read-only into the agent container (K8s ephemeral
    /// provider only). Step volumes come after the provider's.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::{AgentConfig, PodVolumeSource, ReadOnlyVolume};
    ///
    /// let config = AgentConfig::new("review").read_only_volume(ReadOnlyVolume {
    ///     source: PodVolumeSource::PersistentVolumeClaim { claim_name: "repos".to_string() },
    ///     mount_path: "/data/repos/api".to_string(),
    ///     sub_path: Some("api".to_string()),
    /// });
    /// assert_eq!(config.pod.read_only_volumes.len(), 1);
    /// ```
    pub fn read_only_volume(mut self, volume: ReadOnlyVolume) -> Self {
        self.pod.read_only_volumes.push(volume);
        self
    }

    /// Drop the provider's `volume` and `pvc_volume` mounts for this step
    /// (K8s ephemeral provider only). The provider's read-only volumes, its
    /// profiles and its managed settings are kept.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").without_provider_volumes();
    /// assert!(config.pod.without_provider_volumes);
    /// ```
    pub fn without_provider_volumes(mut self) -> Self {
        self.pod.without_provider_volumes = true;
        self
    }

    /// Mount a PersistentVolumeClaim into the agent container, optionally
    /// on a `sub_path` and read-only (K8s ephemeral provider only). Step
    /// mounts are merged after the provider's volumes.
    ///
    /// # Panics
    ///
    /// Panics when the volume is refused by [`PvcVolume::validate`]: an empty
    /// `claim` or a `sub_path` refused by [`validate_pvc_sub_path`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("build")
    ///     .pvc_volume("workspace", "/work", Some("team-a"), false);
    /// assert_eq!(config.pod.pvc_volumes[0].claim_name, "workspace");
    /// assert_eq!(config.pod.pvc_volumes[0].sub_path.as_deref(), Some("team-a"));
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let _ = AgentConfig::new("build").pvc_volume("workspace", "/work", Some("../x"), false);
    /// ```
    pub fn pvc_volume(
        mut self,
        claim: &str,
        mount_path: &str,
        sub_path: Option<&str>,
        read_only: bool,
    ) -> Self {
        let volume = PvcVolume {
            claim_name: claim.to_string(),
            mount_path: mount_path.to_string(),
            sub_path: sub_path.map(str::to_string),
            read_only,
        };
        if let Err(reason) = volume.validate() {
            panic!("invalid pvc_volume: {reason}");
        }
        self.pod.pvc_volumes.push(volume);
        self
    }

    /// Mount a PersistentVolumeClaim read-only at `mount_path` (K8s ephemeral
    /// provider only).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").read_only_pvc("repos", "/data/repos");
    /// assert_eq!(config.pod.read_only_volumes[0].mount_path, "/data/repos");
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

    /// Mount a node directory read-only at `mount_path` (K8s ephemeral
    /// provider only).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").read_only_host_path("/srv/repos", "/data/repos");
    /// assert_eq!(config.pod.read_only_volumes.len(), 1);
    /// ```
    pub fn read_only_host_path(self, host_path: &str, mount_path: &str) -> Self {
        self.read_only_volume(ReadOnlyVolume {
            source: PodVolumeSource::HostPath {
                path: host_path.to_string(),
            },
            mount_path: mount_path.to_string(),
            sub_path: None,
        })
    }

    /// Mount a ConfigMap read-only at `mount_path` (K8s ephemeral provider
    /// only).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").read_only_config_map("guidelines", "/data/guidelines");
    /// assert_eq!(config.pod.read_only_volumes.len(), 1);
    /// ```
    pub fn read_only_config_map(self, name: &str, mount_path: &str) -> Self {
        self.read_only_volume(ReadOnlyVolume {
            source: PodVolumeSource::ConfigMap {
                name: name.to_string(),
            },
            mount_path: mount_path.to_string(),
            sub_path: None,
        })
    }

    /// Select a managed-settings preset registered on the provider (K8s
    /// ephemeral provider only).
    ///
    /// The provider maps the preset to a ConfigMap holding
    /// `managed-settings.json`. An unknown preset fails the step.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").managed_settings("readonly");
    /// assert_eq!(config.pod.managed_settings.as_deref(), Some("readonly"));
    /// ```
    pub fn managed_settings(mut self, preset: &str) -> Self {
        self.pod.managed_settings = Some(preset.to_string());
        self
    }

    /// Select the network egress profile of the pod (K8s providers only).
    ///
    /// Sets the [`LABEL_EGRESS_PROFILE`] pod label, which network policies
    /// select on. Overrides the provider's egress profile.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::{AgentConfig, LABEL_EGRESS_PROFILE};
    ///
    /// let config = AgentConfig::new("open the MR").egress_profile("gitlab");
    /// assert_eq!(config.pod_labels[LABEL_EGRESS_PROFILE], "gitlab");
    /// ```
    pub fn egress_profile(mut self, profile: &str) -> Self {
        self.pod_labels
            .insert(LABEL_EGRESS_PROFILE.to_string(), profile.to_string());
        self
    }

    /// Run the invocation under a Provider Account.
    ///
    /// The worker sets it after selecting an account; providers that support
    /// the account kind inject the credential and record rate-limit windows.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::account::{AccountCredential, AccountSession, RateLimitRecorder};
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let session = AccountSession::new(
    ///     AccountCredential::new("CLAUDE_CODE_OAUTH_TOKEN", "token".to_string()),
    ///     RateLimitRecorder::default(),
    /// );
    /// let config = AgentConfig::new("hello").account_session(session);
    /// assert!(config.account.is_some());
    /// ```
    pub fn account_session(mut self, session: AccountSession) -> Self {
        self.account = Some(session);
        self
    }

    /// Set how long the run may sleep waiting for provider capacity.
    ///
    /// When every targeted account is rate limited, the run sleeps until the
    /// earliest reset if it comes within `wait`, and fails with
    /// [`AgentError::NoCapacity`] otherwise. `Duration::ZERO` fails at once.
    /// Overrides the worker default (6 hours).
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").max_capacity_wait(Duration::ZERO);
    /// assert_eq!(config.max_capacity_wait, Some(Duration::ZERO));
    /// ```
    pub fn max_capacity_wait(mut self, wait: Duration) -> Self {
        self.max_capacity_wait = Some(wait);
        self
    }

    /// Fail the step at once when every targeted account is rate limited.
    ///
    /// Explicit form of [`max_capacity_wait`](Self::max_capacity_wait) with
    /// `Duration::ZERO`: the step fails with [`AgentError::NoCapacity`] instead
    /// of putting the run to sleep, whatever the worker default is.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::time::Duration;
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").fail_fast_on_capacity();
    /// assert_eq!(config.max_capacity_wait, Some(Duration::ZERO));
    /// ```
    pub fn fail_fast_on_capacity(self) -> Self {
        self.max_capacity_wait(Duration::ZERO)
    }

    /// Run the step under the Provider Account named `name`, and only it.
    ///
    /// No failover: when the account is rate limited the run waits (bounded
    /// by [`max_capacity_wait`](Self::max_capacity_wait)), when it does not
    /// exist the step fails with [`AgentError::AccountNotFound`].
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").account("team-a");
    /// assert_eq!(config.account_name.as_deref(), Some("team-a"));
    /// ```
    pub fn account(mut self, name: &str) -> Self {
        self.account_name = Some(name.to_string());
        self
    }

    /// Restrict the step to the Provider Accounts tagged `tag`.
    ///
    /// The strategy picks among them and fails over between them when one
    /// is rate limited during the step.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("review").account_pool("batch");
    /// assert_eq!(config.account_pool.as_deref(), Some("batch"));
    /// ```
    pub fn account_pool(mut self, tag: &str) -> Self {
        self.account_pool = Some(tag.to_string());
        self
    }

    /// Report observed rate-limit windows to `recorder` when the invocation
    /// runs without a Provider Account (the worker environment token).
    ///
    /// Set by the worker; providers prefer the recorder of
    /// [`account`](Self::account) when both are set.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::account::RateLimitRecorder;
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("hello").rate_limit_recorder(RateLimitRecorder::default());
    /// assert!(config.rate_limits.is_some());
    /// ```
    pub fn rate_limit_recorder(mut self, recorder: RateLimitRecorder) -> Self {
        self.rate_limits = Some(recorder);
        self
    }

    /// Record when the step started waiting for provider capacity.
    ///
    /// Set by the engine when a step resumes after a capacity sleep; bounds
    /// the cumulative wait for an account freed by `max_concurrency`.
    ///
    /// # Examples
    ///
    /// ```
    /// use chrono::Utc;
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let since = Utc::now();
    /// let config = AgentConfig::new("hello").capacity_wait_since(since);
    /// assert_eq!(config.capacity_wait_since, Some(since));
    /// ```
    pub fn capacity_wait_since(mut self, since: DateTime<Utc>) -> Self {
        self.capacity_wait_since = Some(since);
        self
    }

    /// Tag the pod with the run id and step name (K8s providers only).
    ///
    /// Sets [`LABEL_RUN_ID`] and [`LABEL_STEP`], both passed through
    /// [`sanitize_label_value`]. The engine sets these labels on every agent
    /// step; call it yourself only when running an agent outside the engine.
    /// The ephemeral provider uses them to delete the pods of a previous
    /// attempt of the same step before starting a new one.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::{AgentConfig, LABEL_RUN_ID, LABEL_STEP};
    ///
    /// let config = AgentConfig::new("investigate").run_scope("demo-run", "investigate");
    /// assert_eq!(config.pod_labels[LABEL_RUN_ID], "demo-run");
    /// assert_eq!(config.pod_labels[LABEL_STEP], "investigate");
    /// ```
    pub fn run_scope(mut self, run_id: &str, step: &str) -> Self {
        self.pod_labels
            .insert(LABEL_RUN_ID.to_string(), sanitize_label_value(run_id));
        self.pod_labels
            .insert(LABEL_STEP.to_string(), sanitize_label_value(step));
        self
    }

    /// Convert to a different typestate by moving all fields.
    ///
    /// Safe because the marker is a zero-sized [`PhantomData`] -- no
    /// runtime data changes.
    fn change_state<T2, S2>(self) -> AgentConfig<T2, S2> {
        AgentConfig {
            system_prompt: self.system_prompt,
            append_system_prompt: self.append_system_prompt,
            prompt: self.prompt,
            model: self.model,
            allowed_tools: self.allowed_tools,
            disallowed_tools: self.disallowed_tools,
            tool_profile: self.tool_profile,
            max_turns: self.max_turns,
            max_parallel_tools: self.max_parallel_tools,
            max_budget_usd: self.max_budget_usd,
            working_dir: self.working_dir,
            mcp_config: self.mcp_config,
            strict_mcp_config: self.strict_mcp_config,
            bare: self.bare,
            permission_mode: self.permission_mode,
            json_schema: self.json_schema,
            resume_session_id: self.resume_session_id,
            session_id: self.session_id,
            resume_prompt: self.resume_prompt,
            resume_environment_id: self.resume_environment_id,
            verbose: self.verbose,
            pod_labels: self.pod_labels,
            pod: self.pod,
            inputs: self.inputs,
            allow_failure: self.allow_failure,
            retry: self.retry,
            trace_context: self.trace_context,
            account: self.account,
            proxied_secrets: self.proxied_secrets,
            max_capacity_wait: self.max_capacity_wait,
            account_name: self.account_name,
            account_pool: self.account_pool,
            rate_limits: self.rate_limits,
            capacity_wait_since: self.capacity_wait_since,
            _marker: PhantomData,
        }
    }
}

// ── allow_tool: only when no schema is set ─────────────────────────

impl<Tools> AgentConfig<Tools, NoSchema> {
    /// Add an allowed tool.
    ///
    /// Can be called multiple times to allow several tools. Returns an
    /// [`AgentConfig<WithTools, NoSchema>`], which **cannot** call
    /// [`output`](AgentConfig::output) or [`output_schema_raw`](AgentConfig::output_schema_raw).
    ///
    /// This restriction exists because Claude CLI has a
    /// [known bug](https://github.com/anthropics/claude-code/issues/18536)
    /// where `--json-schema` combined with `--allowedTools` always returns
    /// `structured_output: null`.
    ///
    /// **Workaround**: use two sequential agent steps -- one with tools to
    /// gather data, then one with `.output::<T>()` to structure the result.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::{AgentConfig, Tool};
    ///
    /// let config = AgentConfig::new("search the web")
    ///     .allow_tool(Tool::WebSearch)
    ///     .allow_tool(Tool::Custom("mcp__docs__lookup".to_string()));
    /// assert_eq!(config.allowed_tools, vec!["WebSearch", "mcp__docs__lookup"]);
    /// ```
    ///
    /// ```compile_fail,E0599
    /// use ironflow_core::provider::{AgentConfig, Tool};
    /// // ERROR: cannot set structured output after adding tools
    /// let _ = AgentConfig::new("x")
    ///     .allow_tool(Tool::Read)
    ///     .output_schema_raw(r#"{"type":"object"}"#);
    /// ```
    pub fn allow_tool(mut self, tool: Tool) -> AgentConfig<WithTools, NoSchema> {
        self.allowed_tools.push(tool.to_string());
        self.change_state()
    }

    /// Select the named tool profile the provider exposes to this step.
    ///
    /// Profiles are registered with
    /// [`HttpAgentProvider::with_tool_profile`](crate::providers::http::HttpAgentProvider::with_tool_profile);
    /// declare each [`ToolProfile`] once as a constant and share it. A profile
    /// the provider does not have fails the step with
    /// [`AgentError::UnknownToolProfile`], never falling back to other tools.
    /// Claude CLI providers fail with [`AgentError::ToolProfileUnsupported`].
    /// Like [`allow_tool`](Self::allow_tool), it rules out
    /// [`output`](AgentConfig::output): split into two steps.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::{AgentConfig, ToolProfile};
    ///
    /// const BUG: ToolProfile = ToolProfile::new("bug");
    ///
    /// let config = AgentConfig::new("Find the root cause").tool_profile(BUG);
    /// assert_eq!(config.tool_profile, Some(BUG));
    /// ```
    ///
    /// ```compile_fail,E0308
    /// use ironflow_core::provider::AgentConfig;
    /// // COMPILE ERROR: a profile is a `ToolProfile`, not a string
    /// let _ = AgentConfig::new("x").tool_profile("bug");
    /// ```
    pub fn tool_profile(mut self, profile: ToolProfile) -> AgentConfig<WithTools, NoSchema> {
        self.tool_profile = Some(profile);
        self.change_state()
    }
}

// ── output: only when no tools are set ─────────────────────────────

impl<Schema> AgentConfig<NoTools, Schema> {
    /// Set structured output from a Rust type implementing [`JsonSchema`].
    ///
    /// The schema is serialized once at build time. When set, the provider
    /// will request typed output conforming to this schema.
    ///
    /// **Important:** structured output requires `max_turns >= 2`.
    ///
    /// Returns an [`AgentConfig<NoTools, WithSchema<T>>`], which remembers
    /// `T` (a workflow step built from it answers with a `T`) and **cannot**
    /// call [`allow_tool`](AgentConfig::allow_tool).
    ///
    /// This restriction exists because Claude CLI has a
    /// [known bug](https://github.com/anthropics/claude-code/issues/18536)
    /// where `--json-schema` combined with `--allowedTools` always returns
    /// `structured_output: null`.
    ///
    /// **Workaround**: use two sequential agent steps -- one with tools to
    /// gather data, then one with `.output::<T>()` to structure the result.
    ///
    /// # Known limitations of Claude CLI structured output
    ///
    /// The Claude CLI does not guarantee strict schema conformance for
    /// structured output. The following upstream bugs affect the behavior:
    ///
    /// - **Schema flattening** ([anthropics/claude-agent-sdk-python#502]):
    ///   a schema like `{"type":"object","properties":{"items":{"type":"array",...}}}`
    ///   may return a bare array instead of the wrapper object. The CLI
    ///   non-deterministically flattens schemas with a single array field.
    /// - **Non-deterministic wrapping** ([anthropics/claude-agent-sdk-python#374]):
    ///   the same prompt can produce differently wrapped output across runs.
    /// - **No conformance guarantee** ([anthropics/claude-code#9058]):
    ///   the CLI does not validate output against the provided JSON schema.
    ///
    /// Because of these bugs, ironflow's provider layer applies multiple
    /// fallback strategies when extracting the structured value (see
    /// [`extract_structured_value`](crate::providers::claude::common::extract_structured_value)).
    ///
    /// [anthropics/claude-agent-sdk-python#502]: https://github.com/anthropics/claude-agent-sdk-python/issues/502
    /// [anthropics/claude-agent-sdk-python#374]: https://github.com/anthropics/claude-agent-sdk-python/issues/374
    /// [anthropics/claude-code#9058]: https://github.com/anthropics/claude-code/issues/9058
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    /// use schemars::JsonSchema;
    ///
    /// #[derive(serde::Deserialize, JsonSchema)]
    /// struct Labels { labels: Vec<String> }
    ///
    /// let config = AgentConfig::new("classify this text")
    ///     .output::<Labels>();
    /// ```
    ///
    /// ```compile_fail,E0599
    /// use ironflow_core::provider::{AgentConfig, Tool};
    /// use schemars::JsonSchema;
    /// #[derive(serde::Deserialize, JsonSchema)]
    /// struct Out { x: i32 }
    /// // ERROR: cannot add tools after setting structured output
    /// let _ = AgentConfig::new("x").output::<Out>().allow_tool(Tool::Read);
    /// ```
    /// # Panics
    ///
    /// Panics if the schema generated by `schemars` cannot be serialized
    /// to JSON. This indicates a bug in the type's `JsonSchema` derive,
    /// not a recoverable runtime error.
    pub fn output<T: JsonSchema>(mut self) -> AgentConfig<NoTools, WithSchema<T>> {
        let schema = schemars::schema_for!(T);
        let serialized = serde_json::to_string(&schema).unwrap_or_else(|e| {
            panic!(
                "failed to serialize JSON schema for {}: {e}",
                std::any::type_name::<T>()
            )
        });
        self.json_schema = Some(serialized);
        self.change_state()
    }

    /// Set structured output from a pre-serialized JSON Schema string.
    ///
    /// Returns an [`AgentConfig<NoTools, RawSchema>`], whose answer is not
    /// typed, and which **cannot** call [`allow_tool`](AgentConfig::allow_tool).
    /// Prefer [`output`](Self::output). See it for the rationale and
    /// workaround.
    pub fn output_schema_raw(mut self, schema: &str) -> AgentConfig<NoTools, RawSchema> {
        self.json_schema = Some(schema.to_string());
        self.change_state()
    }
}

// ── From conversions to base type ──────────────────────────────────

impl From<AgentConfig<WithTools, NoSchema>> for AgentConfig {
    fn from(config: AgentConfig<WithTools, NoSchema>) -> Self {
        config.change_state()
    }
}

impl<T> From<AgentConfig<NoTools, WithSchema<T>>> for AgentConfig {
    fn from(config: AgentConfig<NoTools, WithSchema<T>>) -> Self {
        config.change_state()
    }
}

impl From<AgentConfig<NoTools, RawSchema>> for AgentConfig {
    fn from(config: AgentConfig<NoTools, RawSchema>) -> Self {
        config.change_state()
    }
}

// ── AgentOutput ────────────────────────────────────────────────────

/// Raw output returned by an [`AgentProvider`] after a successful invocation.
///
/// Carries the agent's response value together with usage and billing metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AgentOutput {
    /// The agent's response. A plain [`Value::String`] for text mode, or an
    /// arbitrary JSON value when a JSON schema was requested.
    pub value: Value,

    /// Provider-assigned session identifier, useful for resuming conversations.
    pub session_id: Option<String>,

    /// Total cost in USD for this invocation, if reported by the provider.
    pub cost_usd: Option<f64>,

    /// Uncached input tokens (excludes cache reads and writes), if reported.
    pub input_tokens: Option<u64>,

    /// Input tokens served from the prompt cache, if reported.
    #[serde(default)]
    pub cache_read_input_tokens: Option<u64>,

    /// Input tokens written to the prompt cache, if reported.
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u64>,

    /// Number of output tokens generated, if reported.
    pub output_tokens: Option<u64>,

    /// The concrete model identifier used (e.g. `"claude-sonnet-4-20250514"`).
    pub model: Option<String>,

    /// Wall-clock duration of the invocation in milliseconds.
    pub duration_ms: u64,

    /// Conversation trace captured when [`AgentConfig::verbose`] is `true`.
    ///
    /// Contains every assistant message and tool call made during the
    /// invocation, in chronological order. `None` when verbose mode is off.
    pub debug_messages: Option<Vec<DebugMessage>>,

    /// Identifier of the Provider Account the invocation ran under, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,

    /// Persistent environment the invocation ran in, if any.
    ///
    /// The name of the PersistentVolumeClaim mounted in the agent pod. Only
    /// the K8s ephemeral provider configured with an environment volume sets
    /// it. Pass it to [`AgentConfig::resume_environment`] in a later step to
    /// find the same files again.
    #[serde(default)]
    pub environment_id: Option<String>,
}

/// A single assistant turn captured during a verbose invocation.
///
/// Each `DebugMessage` represents one assistant response, which may contain
/// free-form text, tool calls, or both.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
///
/// # async fn example() -> Result<(), OperationError> {
/// let provider = ClaudeCodeProvider::new();
/// let result = Agent::new()
///     .prompt("List files in src/")
///     .verbose()
///     .run(&provider)
///     .await?;
///
/// if let Some(messages) = result.debug_messages() {
///     for msg in messages {
///         println!("{msg}");
///     }
/// }
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DebugMessage {
    /// Free-form text produced by the assistant in this turn, if any.
    pub text: Option<String>,

    /// Extended thinking blocks produced by the model in this turn.
    ///
    /// Available only when the model emits `thinking` content blocks
    /// (Opus 4.7 adaptive thinking, Claude 3.7+ extended thinking, etc.).
    /// The blocks are joined in arrival order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,

    /// `true` when the model emitted a `thinking` content block but the
    /// text was redacted (only a signature is provided).
    ///
    /// Opus 4.7 adaptive thinking and the `display: "omitted"` setting both
    /// produce signature-only thinking blocks: the model proves it reasoned
    /// without exposing the chain of thought. The UI should still show a
    /// badge so the user knows thinking happened.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub thinking_redacted: bool,

    /// Tool calls made by the assistant in this turn.
    pub tool_calls: Vec<DebugToolCall>,

    /// Tool results received from the user/runtime for the preceding tool calls.
    ///
    /// In the Claude stream-json format, tool results come as `"type":"user"`
    /// messages whose content is a list of `tool_result` blocks. We attach
    /// them to the turn that emitted the matching `tool_use` so the timeline
    /// stays compact.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_results: Vec<DebugToolResult>,

    /// The model's stop reason for this turn (e.g. `"end_turn"`, `"tool_use"`).
    pub stop_reason: Option<String>,

    /// Input tokens consumed by this turn, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,

    /// Output tokens generated by this turn, if reported.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
}

impl fmt::Display for DebugMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref thinking) = self.thinking {
            writeln!(f, "[thinking] {thinking}")?;
        } else if self.thinking_redacted {
            writeln!(f, "[thinking redacted]")?;
        }
        if let Some(ref text) = self.text {
            writeln!(f, "[assistant] {text}")?;
        }
        for tc in &self.tool_calls {
            write!(f, "{tc}")?;
        }
        for tr in &self.tool_results {
            write!(f, "{tr}")?;
        }
        Ok(())
    }
}

/// A single tool call captured during a verbose invocation.
///
/// Records the tool name and its input arguments as a raw JSON value.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DebugToolCall {
    /// Stable identifier assigned by the model (`tool_use_id`).
    ///
    /// Used to correlate a call with its subsequent [`DebugToolResult`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Name of the tool invoked (e.g. `"Read"`, `"Bash"`, `"Grep"`).
    pub name: String,

    /// Input arguments passed to the tool, as raw JSON.
    pub input: Value,
}

impl fmt::Display for DebugToolCall {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  [tool_use] {} -> {}", self.name, self.input)
    }
}

/// A tool result returned to the model after a tool call.
///
/// Carries the tool output (any JSON value: string, object, array) and
/// an error flag if the tool failed.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct DebugToolResult {
    /// The `tool_use_id` this result answers, matching [`DebugToolCall::id`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,

    /// Raw content returned by the tool.
    pub content: Value,

    /// Whether the tool reported an error.
    #[serde(default)]
    pub is_error: bool,
}

impl fmt::Display for DebugToolResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let kind = if self.is_error {
            "tool_error"
        } else {
            "tool_result"
        };
        writeln!(f, "  [{kind}] {}", self.content)
    }
}

impl AgentOutput {
    /// Create an `AgentOutput` with the given value and sensible defaults.
    pub fn new(value: Value) -> Self {
        Self {
            value,
            session_id: None,
            cost_usd: None,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            model: None,
            duration_ms: 0,
            debug_messages: None,
            account_id: None,
            environment_id: None,
        }
    }
}

// ── Log sink ──────────────────────────────────────────────────────

/// Sink for streaming log lines from provider invocations in real time.
///
/// Providers that support live log streaming (e.g. K8s ephemeral) call
/// [`log`](LogSink::log) for each output line as it is produced, enabling
/// downstream consumers (SSE endpoints, log pushers) to display progress
/// before the invocation completes.
///
/// This trait lives in `ironflow-core` so providers can emit logs without
/// depending on higher-level crates.
///
/// # Examples
///
/// ```
/// use std::sync::{Arc, Mutex};
/// use ironflow_core::provider::LogSink;
///
/// struct VecSink(Mutex<Vec<(String, String)>>);
///
/// impl LogSink for VecSink {
///     fn log(&self, stream: &str, line: &str) {
///         self.0.lock().unwrap().push((stream.to_string(), line.to_string()));
///     }
/// }
///
/// let sink = Arc::new(VecSink(Mutex::new(Vec::new())));
/// sink.log("stdout", "hello world");
/// assert_eq!(sink.0.lock().unwrap().len(), 1);
/// ```
pub trait LogSink: Send + Sync {
    /// Emit a single log line on the given stream.
    ///
    /// `stream` is one of `"stdout"`, `"stderr"`, or `"system"`.
    /// Implementations should silently drop lines if the receiver is closed.
    fn log(&self, stream: &str, line: &str);
}

// ── Provider trait ─────────────────────────────────────────────────

/// Trait for AI agent backends.
///
/// Implement this trait to provide a custom AI backend for [`Agent`](crate::operations::agent::Agent).
/// The only required method is [`invoke`](AgentProvider::invoke), which takes an
/// [`AgentConfig`] and returns an [`AgentOutput`] (or an [`AgentError`]).
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
///
/// struct MyProvider;
///
/// impl AgentProvider for MyProvider {
///     fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
///         Box::pin(async move {
///             // Call your custom backend here...
///             todo!()
///         })
///     }
/// }
/// ```
pub trait AgentProvider: Send + Sync {
    /// Execute a single agent invocation with the given configuration.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError`] if the underlying backend process fails,
    /// times out, or produces output that does not match the requested schema.
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a>;

    /// Execute an agent invocation with real-time log streaming.
    ///
    /// Providers that support live output streaming should override this
    /// method to pipe each output line to the [`LogSink`] as it arrives.
    /// The default implementation ignores the sink and delegates to
    /// [`invoke`](AgentProvider::invoke).
    ///
    /// # Errors
    ///
    /// Returns [`AgentError`] if the underlying backend process fails,
    /// times out, or produces output that does not match the requested schema.
    fn invoke_with_logs<'a>(
        &'a self,
        config: &'a AgentConfig,
        log_sink: Arc<dyn LogSink>,
    ) -> InvokeFuture<'a> {
        let _ = log_sink;
        self.invoke(config)
    }

    /// Stop whatever a previous execution of the run `run_id` left running
    /// outside the worker process, before the run executes again.
    ///
    /// The engine calls it before every execution of a run, the first one
    /// included. The default does nothing; the Kubernetes ephemeral provider
    /// deletes the run's pods and waits until they are gone.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError`] when the release fails; the engine then fails
    /// the execution with a replayable error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_core::provider::AgentProvider;
    ///
    /// # async fn example() -> Result<(), ironflow_core::error::AgentError> {
    /// ClaudeCodeProvider::new().release_run("run-1").await?;
    /// # Ok(())
    /// # }
    /// ```
    fn release_run<'a>(&'a self, run_id: &'a str) -> ReleaseFuture<'a> {
        let _ = run_id;
        Box::pin(async { Ok(()) })
    }

    /// The Provider Account kind whose credential this provider can inject.
    ///
    /// `None` (the default) means the provider ignores
    /// [`AgentConfig::account`] and always runs with its own environment.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_core::provider::AgentProvider;
    ///
    /// assert_eq!(ClaudeCodeProvider::new().account_kind(), Some("claude_subscription"));
    /// ```
    fn account_kind(&self) -> Option<&'static str> {
        None
    }

    /// The Provider Account kind of the credential this provider would use
    /// for this specific `config`.
    ///
    /// Defaults to [`AgentProvider::account_kind`]. It differs only for
    /// providers that dispatch to other providers (routers): the kind then
    /// depends on the provider the config is routed to.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    /// use ironflow_core::provider::{AgentConfig, AgentProvider};
    ///
    /// let provider = ClaudeCodeProvider::new();
    /// assert_eq!(
    ///     provider.account_kind_for(&AgentConfig::new("hi")),
    ///     Some("claude_subscription")
    /// );
    /// ```
    fn account_kind_for(&self, _config: &AgentConfig) -> Option<&'static str> {
        self.account_kind()
    }
}

// The decision abstraction lives beside `AgentProvider`: re-exported here so
// `ironflow_core::provider::DecisionProvider` resolves alongside it, while the
// types themselves live in the `decision` module.
pub use crate::decision::{
    ChoiceAnswer, DecideFuture, DecisionAnswer, DecisionOutput, DecisionProvider, DecisionQuestion,
    DecisionRequest, DecisionUsage, NoulAnswer, NoulCriteria, ScoreAnswer,
};

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{from_str, json, to_string};

    use crate::auth_proxy::SecretInjection;

    fn full_config() -> AgentConfig {
        AgentConfig {
            system_prompt: Some("you are helpful".to_string()),
            append_system_prompt: Some("project rules".to_string()),
            prompt: "do stuff".to_string(),
            model: Model::OPUS.to_string(),
            allowed_tools: vec!["Read".to_string(), "Write".to_string()],
            disallowed_tools: vec!["Bash".to_string()],
            tool_profile: None,
            max_turns: Some(10),
            max_parallel_tools: 2,
            max_budget_usd: Some(2.5),
            working_dir: Some("/tmp".to_string()),
            mcp_config: Some("{}".to_string()),
            strict_mcp_config: true,
            bare: true,
            permission_mode: PermissionMode::Auto,
            json_schema: Some(r#"{"type":"object"}"#.to_string()),

            resume_session_id: None,
            session_id: None,
            resume_prompt: None,
            resume_environment_id: None,
            verbose: false,
            pod_labels: BTreeMap::new(),
            pod: PodSettings::default(),
            inputs: Vec::new(),
            allow_failure: false,
            retry: None,
            trace_context: None,
            account: None,
            proxied_secrets: Vec::new(),
            max_capacity_wait: None,
            account_name: None,
            account_pool: None,
            rate_limits: None,
            capacity_wait_since: None,
            _marker: PhantomData,
        }
    }

    #[test]
    fn agent_config_serialize_deserialize_roundtrip() {
        let config = full_config();
        let json = serde_json::to_string(&config).unwrap();
        let back: AgentConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(back.system_prompt, Some("you are helpful".to_string()));
        assert_eq!(back.prompt, "do stuff");
        assert_eq!(back.allowed_tools, vec!["Read", "Write"]);
        assert_eq!(back.max_turns, Some(10));
        assert_eq!(back.max_parallel_tools, 2);
        assert_eq!(back.max_budget_usd, Some(2.5));
        assert_eq!(back.working_dir, Some("/tmp".to_string()));
        assert_eq!(back.mcp_config, Some("{}".to_string()));
        assert_eq!(back.json_schema, Some(r#"{"type":"object"}"#.to_string()));
    }

    #[test]
    fn agent_config_with_all_optional_fields_none() {
        let config: AgentConfig = AgentConfig {
            system_prompt: None,
            append_system_prompt: None,
            prompt: "hello".to_string(),
            model: Model::HAIKU.to_string(),
            allowed_tools: vec![],
            disallowed_tools: vec![],
            tool_profile: None,
            max_turns: None,
            max_parallel_tools: 4,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            strict_mcp_config: false,
            bare: false,
            permission_mode: PermissionMode::Default,
            json_schema: None,

            resume_session_id: None,
            session_id: None,
            resume_prompt: None,
            resume_environment_id: None,
            verbose: false,
            pod_labels: BTreeMap::new(),
            pod: PodSettings::default(),
            inputs: Vec::new(),
            allow_failure: false,
            retry: None,
            trace_context: None,
            account: None,
            proxied_secrets: Vec::new(),
            max_capacity_wait: None,
            account_name: None,
            account_pool: None,
            rate_limits: None,
            capacity_wait_since: None,
            _marker: PhantomData,
        };
        let json = serde_json::to_string(&config).unwrap();
        let back: AgentConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(back.system_prompt, None);
        assert_eq!(back.prompt, "hello");
        assert!(back.allowed_tools.is_empty());
        assert_eq!(back.max_turns, None);
        assert_eq!(back.max_budget_usd, None);
        assert_eq!(back.working_dir, None);
        assert_eq!(back.mcp_config, None);
        assert_eq!(back.json_schema, None);
    }

    #[test]
    fn agent_output_serialize_deserialize_roundtrip() {
        let output = AgentOutput {
            value: json!({"key": "value"}),
            session_id: Some("sess-abc".to_string()),
            cost_usd: Some(0.01),
            input_tokens: Some(500),
            cache_read_input_tokens: Some(4000),
            cache_creation_input_tokens: Some(120),
            output_tokens: Some(200),
            model: Some("claude-sonnet".to_string()),
            duration_ms: 3000,
            debug_messages: None,
            account_id: None,
            environment_id: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        let back: AgentOutput = serde_json::from_str(&json).unwrap();

        assert_eq!(back.value, json!({"key": "value"}));
        assert_eq!(back.session_id, Some("sess-abc".to_string()));
        assert_eq!(back.cost_usd, Some(0.01));
        assert_eq!(back.input_tokens, Some(500));
        assert_eq!(back.cache_read_input_tokens, Some(4000));
        assert_eq!(back.cache_creation_input_tokens, Some(120));
        assert_eq!(back.output_tokens, Some(200));
        assert_eq!(back.model, Some("claude-sonnet".to_string()));
        assert_eq!(back.duration_ms, 3000);
    }

    #[test]
    fn agent_output_deserializes_without_cache_fields() {
        let raw = json!({
            "value": "ok",
            "session_id": null,
            "cost_usd": 0.01,
            "input_tokens": 10,
            "output_tokens": 5,
            "model": null,
            "duration_ms": 100,
            "debug_messages": null
        });
        let back: AgentOutput = serde_json::from_value(raw).unwrap();
        assert_eq!(back.input_tokens, Some(10));
        assert_eq!(back.cache_read_input_tokens, None);
        assert_eq!(back.cache_creation_input_tokens, None);
    }

    #[test]
    fn agent_config_new_has_correct_defaults() {
        let config = AgentConfig::new("test prompt");
        assert_eq!(config.prompt, "test prompt");
        assert_eq!(config.system_prompt, None);
        assert_eq!(config.model, Model::SONNET);
        assert!(config.allowed_tools.is_empty());
        assert_eq!(config.max_turns, None);
        assert_eq!(config.max_budget_usd, None);
        assert_eq!(config.working_dir, None);
        assert_eq!(config.mcp_config, None);
        assert!(matches!(config.permission_mode, PermissionMode::Default));
        assert_eq!(config.json_schema, None);
        assert_eq!(config.resume_session_id, None);
        assert!(!config.verbose);
    }

    #[test]
    fn agent_output_new_has_correct_defaults() {
        let output = AgentOutput::new(json!("test"));
        assert_eq!(output.value, json!("test"));
        assert_eq!(output.session_id, None);
        assert_eq!(output.cost_usd, None);
        assert_eq!(output.input_tokens, None);
        assert_eq!(output.cache_read_input_tokens, None);
        assert_eq!(output.cache_creation_input_tokens, None);
        assert_eq!(output.output_tokens, None);
        assert_eq!(output.model, None);
        assert_eq!(output.duration_ms, 0);
        assert!(output.debug_messages.is_none());
    }

    #[test]
    fn agent_config_resume_session_roundtrip() {
        let mut config = AgentConfig::new("test");
        config.resume_session_id = Some("sess-xyz".to_string());
        let json = serde_json::to_string(&config).unwrap();
        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.resume_session_id, Some("sess-xyz".to_string()));
    }

    #[test]
    fn agent_config_session_id_and_resume_prompt_roundtrip() {
        let config = AgentConfig::new("test")
            .session_id("0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d")
            .resume_prompt("keep going");
        let json = serde_json::to_string(&config).unwrap();
        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.session_id.as_deref(),
            Some("0192f0c1-7d2e-7a4b-9c3d-1e2f3a4b5c6d")
        );
        assert_eq!(back.resume_prompt.as_deref(), Some("keep going"));
    }

    #[test]
    fn agent_config_without_session_id_fields_deserializes() {
        let mut raw = serde_json::to_value(AgentConfig::new("test")).unwrap();
        let object = raw.as_object_mut().unwrap();
        object.remove("session_id");
        object.remove("resume_prompt");
        let back: AgentConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(back.session_id, None);
        assert_eq!(back.resume_prompt, None);
    }

    #[test]
    fn agent_config_session_id_survives_typestate_change() {
        let config: AgentConfig = AgentConfig::new("test")
            .session_id("sid-1")
            .resume_prompt("again")
            .allow_tool(Tool::Read)
            .into();
        assert_eq!(config.session_id.as_deref(), Some("sid-1"));
        assert_eq!(config.resume_prompt.as_deref(), Some("again"));
    }

    #[test]
    fn agent_config_new_has_no_session_id() {
        let config = AgentConfig::new("test");
        assert_eq!(config.session_id, None);
        assert_eq!(config.resume_prompt, None);
    }

    #[test]
    fn agent_config_resume_environment_roundtrip() {
        let config = AgentConfig::new("test").resume_environment("ironflow-env-1");
        let json = serde_json::to_string(&config).unwrap();
        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.resume_environment_id,
            Some("ironflow-env-1".to_string())
        );
    }

    #[test]
    fn agent_config_without_resume_environment_field_deserializes() {
        let mut raw = serde_json::to_value(AgentConfig::new("test")).unwrap();
        raw.as_object_mut().unwrap().remove("resume_environment_id");
        let back: AgentConfig = serde_json::from_value(raw).unwrap();
        assert_eq!(back.resume_environment_id, None);
    }

    #[test]
    fn agent_config_resume_environment_survives_typestate_change() {
        let config: AgentConfig = AgentConfig::new("test")
            .resume_environment("ironflow-env-2")
            .allow_tool(Tool::Read)
            .into();
        assert_eq!(
            config.resume_environment_id.as_deref(),
            Some("ironflow-env-2")
        );
    }

    #[test]
    #[should_panic(expected = "environment_id must not be empty")]
    fn agent_config_resume_environment_empty_panics() {
        let _ = AgentConfig::new("test").resume_environment("");
    }

    #[test]
    fn agent_output_environment_id_roundtrip_and_default() {
        let mut output = AgentOutput::new(json!("ok"));
        output.environment_id = Some("ironflow-env-3".to_string());
        let json = serde_json::to_string(&output).unwrap();
        let back: AgentOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.environment_id, Some("ironflow-env-3".to_string()));

        let raw = json!({
            "value": "ok",
            "session_id": null,
            "cost_usd": null,
            "input_tokens": null,
            "output_tokens": null,
            "model": null,
            "duration_ms": 1,
            "debug_messages": null
        });
        let old: AgentOutput = serde_json::from_value(raw).unwrap();
        assert_eq!(old.environment_id, None);
    }

    #[test]
    fn agent_output_debug_does_not_panic() {
        let output = AgentOutput {
            value: json!(null),
            session_id: None,
            cost_usd: None,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            model: None,
            duration_ms: 0,
            debug_messages: None,
            account_id: None,
            environment_id: None,
        };
        let debug_str = format!("{:?}", output);
        assert!(!debug_str.is_empty());
    }

    #[test]
    fn allow_tool_transitions_to_with_tools() {
        let config = AgentConfig::new("test").allow_tool(Tool::Read);
        assert_eq!(config.allowed_tools, vec!["Read"]);

        // Can add more tools, known or custom.
        let config = config
            .allow_tool(Tool::Write)
            .allow_tool(Tool::Custom("mcp__github__search".to_string()));
        assert_eq!(
            config.allowed_tools,
            vec!["Read", "Write", "mcp__github__search"]
        );
    }

    #[test]
    fn output_carries_the_output_type_in_the_typestate() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Verdict {
            approved: bool,
        }

        let config: AgentConfig<NoTools, WithSchema<Verdict>> =
            AgentConfig::new("review").output::<Verdict>();
        assert!(
            config
                .json_schema
                .as_deref()
                .is_some_and(|s| s.contains("approved"))
        );
    }

    #[test]
    fn output_schema_raw_transitions_to_with_schema() {
        let config = AgentConfig::new("test").output_schema_raw(r#"{"type":"object"}"#);
        assert_eq!(config.json_schema.as_deref(), Some(r#"{"type":"object"}"#));
    }

    #[test]
    fn with_tools_converts_to_base_type() {
        let typed = AgentConfig::new("test").allow_tool(Tool::Read);
        let base: AgentConfig = typed.into();
        assert_eq!(base.allowed_tools, vec!["Read"]);
    }

    #[test]
    fn with_schema_converts_to_base_type() {
        let typed = AgentConfig::new("test").output_schema_raw(r#"{"type":"object"}"#);
        let base: AgentConfig = typed.into();
        assert_eq!(base.json_schema.as_deref(), Some(r#"{"type":"object"}"#));
    }

    #[test]
    fn serde_roundtrip_ignores_marker() {
        let config = AgentConfig::new("test").allow_tool(Tool::Read);
        let json = serde_json::to_string(&config).unwrap();
        assert!(!json.contains("marker"));

        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.allowed_tools, vec!["Read"]);
    }

    #[test]
    fn bare_defaults_to_false() {
        let config = AgentConfig::new("hello");
        assert!(!config.bare, "bare must default to false");
    }

    #[test]
    fn bare_builder_sets_flag() {
        let config = AgentConfig::new("hello").bare(true);
        assert!(config.bare, "bare(true) must enable the flag");

        let config = config.bare(false);
        assert!(!config.bare, "bare(false) must disable the flag");
    }

    #[test]
    fn bare_serde_default_when_missing() {
        let raw = r#"{"prompt":"hello","model":"sonnet"}"#;
        let config: AgentConfig = serde_json::from_str(raw).unwrap();
        assert!(
            !config.bare,
            "bare must default to false when absent from serialized payload"
        );
    }

    #[test]
    fn bare_serde_roundtrip() {
        let mut config = AgentConfig::new("hello");
        config.bare = true;
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            json.contains("\"bare\":true"),
            "serialized form must contain bare:true, got: {json}"
        );

        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert!(back.bare, "bare must survive a serde roundtrip");
    }

    #[test]
    fn disallowed_tools_defaults_to_empty() {
        let config = AgentConfig::new("hello");
        assert!(
            config.disallowed_tools.is_empty(),
            "disallowed_tools must default to empty"
        );
    }

    #[test]
    fn disallowed_tools_builder_replaces_list() {
        let config = AgentConfig::new("hello").disallowed_tools([Tool::Write, Tool::Edit]);
        assert_eq!(config.disallowed_tools, vec!["Write", "Edit"]);

        // Subsequent call fully replaces the list.
        let config = config.disallowed_tools([Tool::Bash]);
        assert_eq!(config.disallowed_tools, vec!["Bash"]);

        // Empty input clears the list.
        let config = config.disallowed_tools([]);
        assert!(config.disallowed_tools.is_empty());
    }

    #[test]
    fn disallowed_tools_compatible_with_output() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Out {
            ok: bool,
        }

        // Typestate compile check: .disallowed_tools(...) must be callable
        // before AND after .output::<T>() because it lives on
        // impl<Tools, Schema>, not impl<Tools, NoSchema>.
        let before: AgentConfig<NoTools, WithSchema<Out>> = AgentConfig::new("classify")
            .disallowed_tools([Tool::Write, Tool::Edit])
            .output::<Out>();
        assert_eq!(before.disallowed_tools, vec!["Write", "Edit"]);
        assert!(before.json_schema.is_some());

        let after: AgentConfig<NoTools, WithSchema<Out>> = AgentConfig::new("classify")
            .output::<Out>()
            .disallowed_tools([Tool::Write]);
        assert_eq!(after.disallowed_tools, vec!["Write"]);
        assert!(after.json_schema.is_some());
    }

    #[test]
    fn disallowed_tools_serde_default_when_missing() {
        let raw = r#"{"prompt":"hello","model":"sonnet"}"#;
        let config: AgentConfig = serde_json::from_str(raw).unwrap();
        assert!(
            config.disallowed_tools.is_empty(),
            "disallowed_tools must default to empty when absent from serialized payload"
        );
    }

    #[test]
    fn disallowed_tools_serde_roundtrip() {
        let config = AgentConfig::new("hello").disallowed_tools([Tool::Write, Tool::Edit]);
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            json.contains("\"disallowed_tools\":[\"Write\",\"Edit\"]"),
            "serialized form must contain the disallowed_tools array, got: {json}"
        );

        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.disallowed_tools, vec!["Write", "Edit"]);
    }

    #[test]
    fn pod_labels_defaults_to_empty() {
        let config = AgentConfig::new("test");
        assert!(config.pod_labels.is_empty());
    }

    #[test]
    fn pod_label_builder_adds_entry() {
        let config = AgentConfig::new("test").pod_label("k", "v");
        assert_eq!(config.pod_labels.len(), 1);
        assert_eq!(config.pod_labels["k"], "v");
    }

    #[test]
    fn pod_labels_builder_replaces_map() {
        let config = AgentConfig::new("test").pod_label("old", "value");
        let mut new_map = BTreeMap::new();
        new_map.insert("new".to_string(), "value".to_string());
        let config = config.pod_labels(new_map);
        assert_eq!(config.pod_labels.len(), 1);
        assert_eq!(config.pod_labels["new"], "value");
        assert!(!config.pod_labels.contains_key("old"));
    }

    #[test]
    fn pod_labels_serde_default_when_missing() {
        let raw = r#"{"prompt":"hello","model":"sonnet"}"#;
        let config: AgentConfig = serde_json::from_str(raw).unwrap();
        assert!(
            config.pod_labels.is_empty(),
            "pod_labels must default to empty when absent from serialized payload"
        );
    }

    #[test]
    fn pod_labels_serde_skip_when_empty() {
        let config = AgentConfig::new("hello");
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            !json.contains("pod_labels"),
            "empty pod_labels must be skipped during serialization, got: {json}"
        );
    }

    #[test]
    fn pod_labels_serde_roundtrip() {
        let config = AgentConfig::new("hello")
            .pod_label("ironflow.io/network-profile", "grafana-only")
            .pod_label("team", "observability");
        let json = serde_json::to_string(&config).unwrap();
        assert!(
            json.contains("pod_labels"),
            "non-empty pod_labels must be present in serialized form, got: {json}"
        );

        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pod_labels.len(), 2);
        assert_eq!(
            back.pod_labels["ironflow.io/network-profile"],
            "grafana-only"
        );
        assert_eq!(back.pod_labels["team"], "observability");
    }

    // ── Pod settings (K8s) ────────────────────────────────────────

    #[test]
    fn k8s_env_from_secret_replaces_same_var() {
        let config = AgentConfig::new("x")
            .env_from_secret("TOKEN", "old-secret", "a")
            .env_from_secret("OTHER", "other", "b")
            .env_from_secret("TOKEN", "new-secret", "c");
        assert_eq!(config.pod.secret_env.len(), 2);
        assert_eq!(config.pod.secret_env[0].name, "TOKEN");
        assert_eq!(config.pod.secret_env[0].secret, "new-secret");
        assert_eq!(config.pod.secret_env[0].key, "c");
        assert_eq!(config.pod.secret_env[1].name, "OTHER");
    }

    fn github_secret(value: &str, host: &str) -> ProxiedSecret {
        ProxiedSecret {
            name: "GITHUB_TOKEN".to_string(),
            env: "GITHUB_TOKEN".to_string(),
            value: value.to_string(),
            injection: SecretInjection::Bearer,
            hosts: vec![host.to_string()],
        }
    }

    #[test]
    fn proxied_secret_replaces_same_env() {
        let gitlab = ProxiedSecret {
            name: "GITLAB_TOKEN".to_string(),
            env: "GITLAB_TOKEN".to_string(),
            value: "glpat-x".to_string(),
            injection: SecretInjection::PrivateToken,
            hosts: vec!["gitlab.com".to_string()],
        };
        let config = AgentConfig::new("x")
            .proxied_secret(github_secret("ghp_old", "api.github.com"))
            .proxied_secret(gitlab)
            .proxied_secret(github_secret("ghp_new", "*.github.com"));
        assert_eq!(config.proxied_secrets.len(), 2);
        assert_eq!(config.proxied_secrets[0].env, "GITHUB_TOKEN");
        assert_eq!(config.proxied_secrets[0].value, "ghp_new");
        assert_eq!(config.proxied_secrets[0].hosts, ["*.github.com"]);
        assert_eq!(config.proxied_secrets[1].env, "GITLAB_TOKEN");
    }

    #[test]
    #[should_panic(expected = "invalid proxied secret")]
    fn proxied_secret_invalid_host_panics() {
        let _ = AgentConfig::new("x").proxied_secret(github_secret("ghp_x", ".*github.com"));
    }

    #[test]
    fn proxied_secret_is_never_serialized() {
        let config = AgentConfig::new("x")
            .proxied_secret(github_secret("ghp_never_serialized", "api.github.com"));
        let json = to_string(&config).unwrap();
        assert!(!json.contains("ghp_never_serialized"), "{json}");
        assert!(!json.contains("proxied_secrets"), "{json}");
        let back: AgentConfig = from_str(&json).unwrap();
        assert!(back.proxied_secrets.is_empty());
    }

    #[test]
    fn k8s_runtime_class_setter_and_serde_round_trip() {
        let config = AgentConfig::new("x").runtime_class("gvisor");
        assert_eq!(config.pod.runtime_class.as_deref(), Some("gvisor"));
        let json = serde_json::to_string(&config).unwrap();
        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pod.runtime_class.as_deref(), Some("gvisor"));
        assert!(AgentConfig::new("x").pod.runtime_class.is_none());
    }

    #[test]
    fn k8s_pod_settings_builders() {
        let config = AgentConfig::new("x")
            .service_account("reader")
            .read_only_pvc("repos", "/data/repos")
            .read_only_host_path("/srv", "/data/srv")
            .read_only_config_map("cm", "/data/cm")
            .managed_settings("locked")
            .egress_profile("gitlab");
        assert_eq!(config.pod.service_account.as_deref(), Some("reader"));
        assert_eq!(config.pod.read_only_volumes.len(), 3);
        assert_eq!(
            config.pod.read_only_volumes[0].source,
            PodVolumeSource::PersistentVolumeClaim {
                claim_name: "repos".to_string(),
            }
        );
        assert_eq!(config.pod.read_only_volumes[0].mount_path, "/data/repos");
        assert_eq!(
            config.pod.read_only_volumes[1].source,
            PodVolumeSource::HostPath {
                path: "/srv".to_string(),
            }
        );
        assert_eq!(
            config.pod.read_only_volumes[2].source,
            PodVolumeSource::ConfigMap {
                name: "cm".to_string(),
            }
        );
        assert_eq!(config.pod.managed_settings.as_deref(), Some("locked"));
        assert_eq!(config.pod_labels[LABEL_EGRESS_PROFILE], "gitlab");
    }

    #[test]
    fn k8s_pvc_volume_builder_pushes_volume() {
        let config = AgentConfig::new("x")
            .pvc_volume("ws", "/work", Some("a/b"), true)
            .pvc_volume("ws", "/other", None, false);
        assert_eq!(config.pod.pvc_volumes.len(), 2);
        assert_eq!(config.pod.pvc_volumes[0].claim_name, "ws");
        assert_eq!(config.pod.pvc_volumes[0].mount_path, "/work");
        assert_eq!(config.pod.pvc_volumes[0].sub_path.as_deref(), Some("a/b"));
        assert!(config.pod.pvc_volumes[0].read_only);
        assert_eq!(config.pod.pvc_volumes[1].sub_path, None);
        assert!(!config.pod.pvc_volumes[1].read_only);
    }

    #[test]
    fn k8s_without_provider_volumes_sets_flag() {
        assert!(!AgentConfig::new("x").pod.without_provider_volumes);
        let config = AgentConfig::new("x").without_provider_volumes();
        assert!(config.pod.without_provider_volumes);
    }

    #[test]
    #[should_panic(expected = "sub_path")]
    fn k8s_pvc_volume_rejects_parent_sub_path() {
        let _ = AgentConfig::new("x").pvc_volume("ws", "/work", Some("../x"), false);
    }

    #[test]
    #[should_panic(expected = "sub_path")]
    fn k8s_pvc_volume_rejects_absolute_sub_path() {
        let _ = AgentConfig::new("x").pvc_volume("ws", "/work", Some("/abs"), false);
    }

    #[test]
    #[should_panic(expected = "sub_path")]
    fn k8s_pvc_volume_rejects_empty_segment_sub_path() {
        let _ = AgentConfig::new("x").pvc_volume("ws", "/work", Some("a//b"), false);
    }

    #[test]
    #[should_panic(expected = "claim_name")]
    fn k8s_pvc_volume_rejects_empty_claim() {
        let _ = AgentConfig::new("x").pvc_volume("", "/work", None, false);
    }

    #[test]
    fn k8s_run_scope_sets_sanitized_labels() {
        let config = AgentConfig::new("x").run_scope("run-1", "fix bug/42");
        assert_eq!(config.pod_labels[LABEL_RUN_ID], "run-1");
        assert_eq!(
            config.pod_labels[LABEL_STEP],
            sanitize_label_value("fix bug/42")
        );
        assert!(config.pod_labels[LABEL_STEP].starts_with("fix-bug-42-"));
    }

    #[test]
    fn k8s_pod_settings_serde_skip_when_empty() {
        let json = serde_json::to_value(AgentConfig::new("hello")).unwrap();
        assert!(json.get("pod").is_none(), "empty pod must be skipped");
    }

    #[test]
    fn k8s_pod_settings_serde_roundtrip() {
        let config = AgentConfig::new("hello")
            .env_from_secret("TOKEN", "s", "k")
            .service_account("sa")
            .read_only_pvc("repos", "/data/repos")
            .managed_settings("locked");
        let json = serde_json::to_string(&config).unwrap();
        let back: AgentConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back.pod, config.pod);
    }

    #[test]
    fn k8s_pod_settings_serde_default_when_missing() {
        let raw = r#"{"prompt":"hello","model":"sonnet"}"#;
        let config: AgentConfig = serde_json::from_str(raw).unwrap();
        assert!(config.pod.is_empty());
    }

    // ── LogSink tests ─────────────────────────────────────────────

    use crate::test_support::VecSink;

    #[test]
    fn log_sink_collects_lines() {
        let sink = VecSink::new();
        sink.log("stdout", "line 1");
        sink.log("stderr", "err!");
        sink.log("system", "done");

        let lines = sink.0.lock().unwrap();
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[0], ("stdout".to_string(), "line 1".to_string()));
        assert_eq!(lines[1], ("stderr".to_string(), "err!".to_string()));
        assert_eq!(lines[2], ("system".to_string(), "done".to_string()));
    }

    #[test]
    fn log_sink_arc_is_clone_and_send() {
        let sink: Arc<dyn LogSink> = VecSink::new();
        let cloned = sink.clone();
        sink.log("stdout", "from original");
        cloned.log("stdout", "from clone");
    }

    // ── invoke_with_logs default impl ─────────────────────────────

    struct FixedProvider {
        output: AgentOutput,
    }

    impl AgentProvider for FixedProvider {
        fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
            Box::pin(async {
                Ok(AgentOutput {
                    value: self.output.value.clone(),
                    session_id: self.output.session_id.clone(),
                    cost_usd: self.output.cost_usd,
                    input_tokens: self.output.input_tokens,
                    cache_read_input_tokens: self.output.cache_read_input_tokens,
                    cache_creation_input_tokens: self.output.cache_creation_input_tokens,
                    output_tokens: self.output.output_tokens,
                    model: self.output.model.clone(),
                    duration_ms: self.output.duration_ms,
                    debug_messages: None,
                    account_id: None,
                    environment_id: None,
                })
            })
        }
    }

    #[tokio::test]
    async fn release_run_default_does_nothing() {
        let provider = FixedProvider {
            output: AgentOutput::new(json!("ok")),
        };
        assert!(provider.release_run("run-1").await.is_ok());
        assert!(provider.release_run("").await.is_ok());
    }

    #[tokio::test]
    async fn invoke_with_logs_default_delegates_to_invoke() {
        let provider = FixedProvider {
            output: AgentOutput::new(json!("ok")),
        };
        let config = AgentConfig::new("test");
        let sink: Arc<dyn LogSink> = VecSink::new();

        let result = provider.invoke_with_logs(&config, sink.clone()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap().value, json!("ok"));
    }

    #[tokio::test]
    async fn invoke_with_logs_default_ignores_sink() {
        let provider = FixedProvider {
            output: AgentOutput::new(json!("ok")),
        };
        let config = AgentConfig::new("test");
        let sink = VecSink::new();

        let _ = provider
            .invoke_with_logs(&config, sink.clone() as Arc<dyn LogSink>)
            .await;

        let lines = sink.0.lock().unwrap();
        assert!(lines.is_empty(), "default impl should not emit any logs");
    }

    #[test]
    fn max_parallel_tools_defaults_to_four() {
        assert_eq!(AgentConfig::new("hi").max_parallel_tools, 4);
    }

    #[test]
    fn max_parallel_tools_builder_sets_value() {
        let config = AgentConfig::new("hi").max_parallel_tools(2);
        assert_eq!(config.max_parallel_tools, 2);
    }

    #[test]
    #[should_panic(expected = "max_parallel_tools must be greater than 0")]
    fn max_parallel_tools_zero_panics() {
        let _ = AgentConfig::new("hi").max_parallel_tools(0);
    }

    #[test]
    fn max_parallel_tools_missing_from_json_defaults_to_four() {
        let json = serde_json::to_value(AgentConfig::new("hi")).unwrap();
        let mut obj = json.as_object().unwrap().clone();
        obj.remove("max_parallel_tools");
        let back: AgentConfig = serde_json::from_value(Value::Object(obj)).unwrap();
        assert_eq!(back.max_parallel_tools, 4);
    }
}
