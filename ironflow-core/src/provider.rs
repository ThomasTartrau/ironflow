//! Provider trait and configuration types for agent invocations.
//!
//! The [`AgentProvider`] trait is the primary extension point in ironflow: implement it
//! to plug in any AI backend (local model, HTTP API, mock, etc.) without changing
//! your workflow code.
//!
//! The built-in implementations are:
//!
//! * [`ClaudeCodeProvider`](crate::providers::claude::ClaudeCodeProvider) - shells out
//!   to the `claude` CLI.
//! * [`RecordReplayProvider`](crate::providers::record_replay::RecordReplayProvider) -
//!   records and replays fixtures for deterministic testing.

use std::future::Future;
use std::pin::Pin;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::AgentError;
use crate::operations::agent::{Model, PermissionMode};

/// Boxed future returned by [`AgentProvider::invoke`].
pub type InvokeFuture<'a> =
    Pin<Box<dyn Future<Output = Result<AgentOutput, AgentError>> + Send + 'a>>;

/// Serializable configuration passed to an [`AgentProvider`] for a single invocation.
///
/// Built by [`Agent::run`](crate::operations::agent::Agent::run) from the builder state.
/// Provider implementations translate these fields into whatever format the underlying
/// backend expects.
#[derive(Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct AgentConfig {
    /// Optional system prompt that sets the agent's persona or constraints.
    pub system_prompt: Option<String>,

    /// The user prompt - the main instruction to the agent.
    pub prompt: String,

    /// Which model to use for this invocation.
    pub model: Model,

    /// Allowlist of tool names the agent may invoke (empty = provider default).
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
    pub permission_mode: PermissionMode,

    /// Optional JSON Schema string. When set, the provider should request
    /// structured (typed) output from the model.
    pub json_schema: Option<String>,

    /// Optional session ID to resume a previous conversation.
    ///
    /// When set, the provider should continue the conversation from the
    /// specified session rather than starting a new one.
    pub resume_session_id: Option<String>,
}

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
}

impl AgentConfig {
    /// Create an `AgentConfig` with required fields and defaults for the rest.
    pub fn new(prompt: &str) -> Self {
        Self {
            system_prompt: None,
            prompt: prompt.to_string(),
            model: Model::Sonnet,
            allowed_tools: Vec::new(),
            max_turns: None,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            permission_mode: PermissionMode::Default,
            json_schema: None,
            resume_session_id: None,
        }
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
        }
    }
}

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
            model: Model::Opus,
            allowed_tools: vec!["Read".to_string(), "Write".to_string()],
            max_turns: Some(10),
            max_budget_usd: Some(2.5),
            working_dir: Some("/tmp".to_string()),
            mcp_config: Some("{}".to_string()),
            permission_mode: PermissionMode::Auto,
            json_schema: Some(r#"{"type":"object"}"#.to_string()),
            resume_session_id: None,
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
        let config = AgentConfig {
            system_prompt: None,
            prompt: "hello".to_string(),
            model: Model::Haiku,
            allowed_tools: vec![],
            max_turns: None,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            permission_mode: PermissionMode::Default,
            json_schema: None,
            resume_session_id: None,
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
        assert!(matches!(config.model, Model::Sonnet));
        assert!(config.allowed_tools.is_empty());
        assert_eq!(config.max_turns, None);
        assert_eq!(config.max_budget_usd, None);
        assert_eq!(config.working_dir, None);
        assert_eq!(config.mcp_config, None);
        assert!(matches!(config.permission_mode, PermissionMode::Default));
        assert_eq!(config.json_schema, None);
        assert_eq!(config.resume_session_id, None);
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
        };
        let debug_str = format!("{:?}", output);
        assert!(!debug_str.is_empty());
    }
}
