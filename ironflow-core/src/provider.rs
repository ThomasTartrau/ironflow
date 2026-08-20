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

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AgentError;
use crate::operations::agent::{Model, PermissionMode};
use crate::retry::RetryPolicy;

/// Boxed future returned by [`AgentProvider::invoke`].
pub type InvokeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AgentOutput, AgentError>> + Send + 'a>>;

// ── Typestate markers ──────────────────────────────────────────────

/// Marker: no tools have been added via the builder.
#[derive(Debug, Clone, Copy)]
pub struct NoTools;

/// Marker: at least one tool has been added via [`AgentConfig::allow_tool`].
#[derive(Debug, Clone, Copy)]
pub struct WithTools;

/// Marker: no JSON schema has been set via the builder.
#[derive(Debug, Clone, Copy)]
pub struct NoSchema;

/// Marker: a JSON schema has been set via [`AgentConfig::output`] or
/// [`AgentConfig::output_schema_raw`].
#[derive(Debug, Clone, Copy)]
pub struct WithSchema;

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
/// use ironflow_core::provider::AgentConfig;
///
/// // OK: tools only
/// let _ = AgentConfig::new("search").allow_tool("WebSearch");
///
/// // OK: structured output only
/// let _ = AgentConfig::new("classify").output_schema_raw(r#"{"type":"object"}"#);
/// ```
///
/// ```compile_fail
/// use ironflow_core::provider::AgentConfig;
/// // COMPILE ERROR: cannot add tools after setting structured output
/// let _ = AgentConfig::new("x").output_schema_raw("{}").allow_tool("Read");
/// ```
///
/// ```compile_fail
/// use ironflow_core::provider::AgentConfig;
/// // COMPILE ERROR: cannot set structured output after adding tools
/// let _ = AgentConfig::new("x").allow_tool("Read").output_schema_raw("{}");
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

    /// Maximum number of agentic turns before the provider should stop.
    pub max_turns: Option<u32>,

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

    /// Zero-sized typestate marker (not serialized).
    #[serde(skip)]
    pub(crate) _marker: PhantomData<(Tools, Schema)>,
}

fn default_model() -> String {
    Model::SONNET.to_string()
}

// ── Constructor (base type only) ───────────────────────────────────

impl AgentConfig {
    /// Create an `AgentConfig` with required fields and defaults for the rest.
    pub fn new(prompt: &str) -> Self {
        Self {
            system_prompt: None,
            prompt: prompt.to_string(),
            model: Model::SONNET.to_string(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            max_turns: None,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            strict_mcp_config: false,
            bare: false,
            permission_mode: PermissionMode::Default,
            json_schema: None,

            resume_session_id: None,
            verbose: false,
            pod_labels: BTreeMap::new(),
            inputs: Vec::new(),
            allow_failure: false,
            retry: None,
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
    /// [`AgentConfig<NoTools, WithSchema>`]) because, unlike
    /// [`allow_tool`](AgentConfig::allow_tool), `disallowed_tools` does not
    /// activate any tool -- it only filters out tools that would otherwise be
    /// loaded by default.
    ///
    /// As such, it is safe to combine with structured output:
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
    /// let config = AgentConfig::new("classify this")
    ///     .disallowed_tools(["Write", "Edit"])
    ///     .output::<Out>();
    /// ```
    pub fn disallowed_tools<I, S>(mut self, tools: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.disallowed_tools = tools.into_iter().map(Into::into).collect();
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
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("Read /work/dossier.pdf and summarize")
    ///     .allow_tool("Read")
    ///     .input_file("https://r2.example.com/dossier.pdf", "/work/dossier.pdf");
    /// ```
    pub fn input_file(mut self, url: &str, mount_path: &str) -> Self {
        self.inputs.push(AgentInput::new(url, mount_path));
        self
    }

    /// Convert to a different typestate by moving all fields.
    ///
    /// Safe because the marker is a zero-sized [`PhantomData`] -- no
    /// runtime data changes.
    fn change_state<T2, S2>(self) -> AgentConfig<T2, S2> {
        AgentConfig {
            system_prompt: self.system_prompt,
            prompt: self.prompt,
            model: self.model,
            allowed_tools: self.allowed_tools,
            disallowed_tools: self.disallowed_tools,
            max_turns: self.max_turns,
            max_budget_usd: self.max_budget_usd,
            working_dir: self.working_dir,
            mcp_config: self.mcp_config,
            strict_mcp_config: self.strict_mcp_config,
            bare: self.bare,
            permission_mode: self.permission_mode,
            json_schema: self.json_schema,
            resume_session_id: self.resume_session_id,
            verbose: self.verbose,
            pod_labels: self.pod_labels,
            inputs: self.inputs,
            allow_failure: self.allow_failure,
            retry: self.retry,
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
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("search the web")
    ///     .allow_tool("WebSearch")
    ///     .allow_tool("WebFetch");
    /// ```
    ///
    /// ```compile_fail
    /// use ironflow_core::provider::AgentConfig;
    /// // ERROR: cannot set structured output after adding tools
    /// let _ = AgentConfig::new("x")
    ///     .allow_tool("Read")
    ///     .output_schema_raw(r#"{"type":"object"}"#);
    /// ```
    pub fn allow_tool(mut self, tool: &str) -> AgentConfig<WithTools, NoSchema> {
        self.allowed_tools.push(tool.to_string());
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
    /// Returns an [`AgentConfig<NoTools, WithSchema>`], which **cannot**
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
    /// ```compile_fail
    /// use ironflow_core::provider::AgentConfig;
    /// use schemars::JsonSchema;
    /// #[derive(serde::Deserialize, JsonSchema)]
    /// struct Out { x: i32 }
    /// // ERROR: cannot add tools after setting structured output
    /// let _ = AgentConfig::new("x").output::<Out>().allow_tool("Read");
    /// ```
    /// # Panics
    ///
    /// Panics if the schema generated by `schemars` cannot be serialized
    /// to JSON. This indicates a bug in the type's `JsonSchema` derive,
    /// not a recoverable runtime error.
    pub fn output<T: JsonSchema>(mut self) -> AgentConfig<NoTools, WithSchema> {
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
    /// Returns an [`AgentConfig<NoTools, WithSchema>`], which **cannot**
    /// call [`allow_tool`](AgentConfig::allow_tool). See [`output`](Self::output)
    /// for the rationale and workaround.
    pub fn output_schema_raw(mut self, schema: &str) -> AgentConfig<NoTools, WithSchema> {
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

impl From<AgentConfig<NoTools, WithSchema>> for AgentConfig {
    fn from(config: AgentConfig<NoTools, WithSchema>) -> Self {
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

    /// Number of input tokens consumed, if reported.
    pub input_tokens: Option<u64>,

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
            output_tokens: None,
            model: None,
            duration_ms: 0,
            debug_messages: None,
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn full_config() -> AgentConfig {
        AgentConfig {
            system_prompt: Some("you are helpful".to_string()),
            prompt: "do stuff".to_string(),
            model: Model::OPUS.to_string(),
            allowed_tools: vec!["Read".to_string(), "Write".to_string()],
            disallowed_tools: vec!["Bash".to_string()],
            max_turns: Some(10),
            max_budget_usd: Some(2.5),
            working_dir: Some("/tmp".to_string()),
            mcp_config: Some("{}".to_string()),
            strict_mcp_config: true,
            bare: true,
            permission_mode: PermissionMode::Auto,
            json_schema: Some(r#"{"type":"object"}"#.to_string()),

            resume_session_id: None,
            verbose: false,
            pod_labels: BTreeMap::new(),
            inputs: Vec::new(),
            allow_failure: false,
            retry: None,
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
        assert_eq!(back.max_budget_usd, Some(2.5));
        assert_eq!(back.working_dir, Some("/tmp".to_string()));
        assert_eq!(back.mcp_config, Some("{}".to_string()));
        assert_eq!(back.json_schema, Some(r#"{"type":"object"}"#.to_string()));
    }

    #[test]
    fn agent_config_with_all_optional_fields_none() {
        let config: AgentConfig = AgentConfig {
            system_prompt: None,
            prompt: "hello".to_string(),
            model: Model::HAIKU.to_string(),
            allowed_tools: vec![],
            disallowed_tools: vec![],
            max_turns: None,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            strict_mcp_config: false,
            bare: false,
            permission_mode: PermissionMode::Default,
            json_schema: None,

            resume_session_id: None,
            verbose: false,
            pod_labels: BTreeMap::new(),
            inputs: Vec::new(),
            allow_failure: false,
            retry: None,
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
            output_tokens: Some(200),
            model: Some("claude-sonnet".to_string()),
            duration_ms: 3000,
            debug_messages: None,
        };
        let json = serde_json::to_string(&output).unwrap();
        let back: AgentOutput = serde_json::from_str(&json).unwrap();

        assert_eq!(back.value, json!({"key": "value"}));
        assert_eq!(back.session_id, Some("sess-abc".to_string()));
        assert_eq!(back.cost_usd, Some(0.01));
        assert_eq!(back.input_tokens, Some(500));
        assert_eq!(back.output_tokens, Some(200));
        assert_eq!(back.model, Some("claude-sonnet".to_string()));
        assert_eq!(back.duration_ms, 3000);
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
    fn agent_output_debug_does_not_panic() {
        let output = AgentOutput {
            value: json!(null),
            session_id: None,
            cost_usd: None,
            input_tokens: None,
            output_tokens: None,
            model: None,
            duration_ms: 0,
            debug_messages: None,
        };
        let debug_str = format!("{:?}", output);
        assert!(!debug_str.is_empty());
    }

    #[test]
    fn allow_tool_transitions_to_with_tools() {
        let config = AgentConfig::new("test").allow_tool("Read");
        assert_eq!(config.allowed_tools, vec!["Read"]);

        // Can add more tools
        let config = config.allow_tool("Write");
        assert_eq!(config.allowed_tools, vec!["Read", "Write"]);
    }

    #[test]
    fn output_schema_raw_transitions_to_with_schema() {
        let config = AgentConfig::new("test").output_schema_raw(r#"{"type":"object"}"#);
        assert_eq!(config.json_schema.as_deref(), Some(r#"{"type":"object"}"#));
    }

    #[test]
    fn with_tools_converts_to_base_type() {
        let typed = AgentConfig::new("test").allow_tool("Read");
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
        let config = AgentConfig::new("test").allow_tool("Read");
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
        let config = AgentConfig::new("hello").disallowed_tools(["Write", "Edit"]);
        assert_eq!(config.disallowed_tools, vec!["Write", "Edit"]);

        // Subsequent call fully replaces the list.
        let config = config.disallowed_tools(["Bash"]);
        assert_eq!(config.disallowed_tools, vec!["Bash"]);

        // Empty input clears the list.
        let config = config.disallowed_tools(std::iter::empty::<String>());
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
        let before: AgentConfig<NoTools, WithSchema> = AgentConfig::new("classify")
            .disallowed_tools(["Write", "Edit"])
            .output::<Out>();
        assert_eq!(before.disallowed_tools, vec!["Write", "Edit"]);
        assert!(before.json_schema.is_some());

        let after: AgentConfig<NoTools, WithSchema> = AgentConfig::new("classify")
            .output::<Out>()
            .disallowed_tools(["Write"]);
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
        let config = AgentConfig::new("hello").disallowed_tools(["Write", "Edit"]);
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
                    output_tokens: self.output.output_tokens,
                    model: self.output.model.clone(),
                    duration_ms: self.output.duration_ms,
                    debug_messages: None,
                })
            })
        }
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
}
