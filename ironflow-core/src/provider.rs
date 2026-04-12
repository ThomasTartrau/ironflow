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

use std::fmt;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

use crate::error::AgentError;
use crate::operations::agent::{Model, PermissionMode};

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

    /// Maximum number of agentic turns before the provider should stop.
    pub max_turns: Option<u32>,

    /// Maximum spend in USD for this single invocation.
    pub max_budget_usd: Option<f64>,

    /// Working directory for the agent process.
    pub working_dir: Option<String>,

    /// Path to an MCP server configuration file.
    pub mcp_config: Option<String>,

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
            max_turns: None,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            permission_mode: PermissionMode::Default,
            json_schema: None,
            resume_session_id: None,
            verbose: false,
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

    /// Set a session ID to resume a previous conversation.
    pub fn resume(mut self, session_id: &str) -> Self {
        self.resume_session_id = Some(session_id.to_string());
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
            max_turns: self.max_turns,
            max_budget_usd: self.max_budget_usd,
            working_dir: self.working_dir,
            mcp_config: self.mcp_config,
            permission_mode: self.permission_mode,
            json_schema: self.json_schema,
            resume_session_id: self.resume_session_id,
            verbose: self.verbose,
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
    pub fn output<T: JsonSchema>(mut self) -> AgentConfig<NoTools, WithSchema> {
        let schema = schemars::schema_for!(T);
        self.json_schema = match serde_json::to_string(&schema) {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(
                    error = %e,
                    type_name = std::any::type_name::<T>(),
                    "failed to serialize JSON schema, structured output disabled"
                );
                None
            }
        };
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

    /// Tool calls made by the assistant in this turn.
    pub tool_calls: Vec<DebugToolCall>,

    /// The model's stop reason for this turn (e.g. `"end_turn"`, `"tool_use"`).
    pub stop_reason: Option<String>,
}

impl fmt::Display for DebugMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(ref text) = self.text {
            writeln!(f, "[assistant] {text}")?;
        }
        for tc in &self.tool_calls {
            write!(f, "{tc}")?;
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
            max_turns: Some(10),
            max_budget_usd: Some(2.5),
            working_dir: Some("/tmp".to_string()),
            mcp_config: Some("{}".to_string()),
            permission_mode: PermissionMode::Auto,
            json_schema: Some(r#"{"type":"object"}"#.to_string()),
            resume_session_id: None,
            verbose: false,
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
            max_turns: None,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            permission_mode: PermissionMode::Default,
            json_schema: None,
            resume_session_id: None,
            verbose: false,
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
}
