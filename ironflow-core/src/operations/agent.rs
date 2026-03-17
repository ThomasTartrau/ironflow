//! Agent operation - build and execute AI agent calls.
//!
//! The [`Agent`] builder lets you configure a single agent invocation (model,
//! prompt, tools, budget, permissions, etc.) and execute it through any
//! [`AgentProvider`]. The result is an [`AgentResult`] that provides typed
//! access to the agent's response, session metadata, and usage statistics.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let provider = ClaudeCodeProvider::new();
//!
//! let result = Agent::new()
//!     .prompt("Summarize the README.md file")
//!     .model(Model::Sonnet)
//!     .max_turns(3)
//!     .run(&provider)
//!     .await?;
//!
//! println!("{}", result.text());
//! # Ok(())
//! # }
//! ```

use std::fmt::{self, Display};

use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_value, to_string};
use tracing::{info, warn};

use crate::error::OperationError;
#[cfg(feature = "prometheus")]
use crate::metric_names;
use crate::provider::{AgentConfig, AgentOutput, AgentProvider};

/// Available Claude models.
///
/// Each variant maps to a model name passed to the Claude CLI.
/// Use the alias variants (`Sonnet`, `Opus`, `Haiku`) for the latest version,
/// or the explicit variants for a specific model ID and context window.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    // ── Aliases (latest version, CLI resolves to current) ───────────
    /// Claude Sonnet - balanced speed and capability (default, 200K context).
    Sonnet,
    /// Claude Opus - highest capability (200K context).
    Opus,
    /// Claude Haiku - fastest and cheapest (200K context).
    Haiku,

    // ── Claude 4.5 ─────────────────────────────────────────────────
    /// Claude Haiku 4.5 - 200K context.
    #[serde(rename = "claude-haiku-4-5-20251001")]
    Haiku45,

    // ── Claude 4.6 - 200K context ──────────────────────────────────
    /// Claude Sonnet 4.6 - 200K context.
    #[serde(rename = "claude-sonnet-4-6")]
    Sonnet46,
    /// Claude Opus 4.6 - 200K context.
    #[serde(rename = "claude-opus-4-6")]
    Opus46,

    // ── Claude 4.6 - 1M context ────────────────────────────────────
    /// Claude Sonnet 4.6 with 1M token context window.
    #[serde(rename = "claude-sonnet-4-6[1m]")]
    Sonnet46_1M,
    /// Claude Opus 4.6 with 1M token context window.
    #[serde(rename = "claude-opus-4-6[1m]")]
    Opus46_1M,
}

impl Model {
    /// Maximum context window size in tokens.
    pub fn context_window(self) -> usize {
        match self {
            Self::Sonnet46_1M | Self::Opus46_1M => 1_000_000,
            _ => 200_000,
        }
    }
}

impl Display for Model {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match serde_json::to_value(self) {
            Ok(Value::String(s)) => f.write_str(&s),
            _ => f.write_str("unknown"),
        }
    }
}

/// Controls how the agent handles tool-use permission prompts.
///
/// These map to the `--permission-mode` and `--dangerously-skip-permissions`
/// flags in the Claude CLI.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum PermissionMode {
    /// Use the CLI default permission behavior.
    Default,
    /// Automatically approve tool-use requests.
    Auto,
    /// Suppress all permission prompts (the agent proceeds without asking).
    DontAsk,
    /// Skip all permission checks entirely.
    ///
    /// **Warning**: the agent will have unrestricted filesystem and shell access.
    BypassPermissions,
}

/// Builder for a single agent invocation.
///
/// Create with [`Agent::new`], chain configuration methods, then call
/// [`run`](Agent::run) with an [`AgentProvider`] to execute.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
///
/// # async fn example() -> Result<(), OperationError> {
/// let provider = ClaudeCodeProvider::new();
///
/// let result = Agent::new()
///     .system_prompt("You are a Rust expert.")
///     .prompt("Review this code for safety issues.")
///     .model(Model::Opus)
///     .allowed_tools(&["Read", "Grep"])
///     .max_turns(5)
///     .max_budget_usd(0.50)
///     .working_dir("/tmp/project")
///     .permission_mode(PermissionMode::Auto)
///     .run(&provider)
///     .await?;
///
/// println!("Cost: ${:.4}", result.cost_usd().unwrap_or(0.0));
/// # Ok(())
/// # }
/// ```
#[must_use = "an Agent does nothing until .run() is awaited"]
pub struct Agent {
    config: AgentConfig,
    dry_run: Option<bool>,
}

impl Agent {
    /// Create a new agent builder with default settings.
    ///
    /// Defaults: [`Model::Sonnet`], no system prompt, no tool restrictions,
    /// no budget/turn limits, [`PermissionMode::Default`].
    pub fn new() -> Self {
        Self {
            config: AgentConfig::new(""),
            dry_run: None,
        }
    }

    /// Set the system prompt that defines the agent's persona or constraints.
    pub fn system_prompt(mut self, prompt: &str) -> Self {
        self.config.system_prompt = Some(prompt.to_string());
        self
    }

    /// Set the user prompt - the main instruction sent to the agent.
    pub fn prompt(mut self, prompt: &str) -> Self {
        self.config.prompt = prompt.to_string();
        self
    }

    /// Set the model tier to use for this invocation.
    ///
    /// Defaults to [`Model::Sonnet`] if not called.
    pub fn model(mut self, model: Model) -> Self {
        self.config.model = model;
        self
    }

    /// Restrict which tools the agent may invoke.
    ///
    /// Pass an empty slice (or do not call this method) to allow the provider
    /// default set of tools.
    pub fn allowed_tools(mut self, tools: &[&str]) -> Self {
        self.config.allowed_tools = tools.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Set the maximum number of agentic turns.
    ///
    /// # Panics
    ///
    /// Panics if `turns` is `0`.
    pub fn max_turns(mut self, turns: u32) -> Self {
        assert!(turns > 0, "max_turns must be greater than 0");
        self.config.max_turns = Some(turns);
        self
    }

    /// Set the maximum spend in USD for this invocation.
    ///
    /// # Panics
    ///
    /// Panics if `budget` is negative, NaN, or infinity.
    pub fn max_budget_usd(mut self, budget: f64) -> Self {
        assert!(
            budget.is_finite() && budget > 0.0,
            "budget must be a positive finite number, got {budget}"
        );
        self.config.max_budget_usd = Some(budget);
        self
    }

    /// Set the working directory for the agent process.
    pub fn working_dir(mut self, dir: &str) -> Self {
        self.config.working_dir = Some(dir.to_string());
        self
    }

    /// Set the path to an MCP (Model Context Protocol) server configuration file.
    pub fn mcp_config(mut self, config: &str) -> Self {
        self.config.mcp_config = Some(config.to_string());
        self
    }

    /// Set the permission mode controlling tool-use approval behavior.
    ///
    /// See [`PermissionMode`] for details on each variant.
    pub fn permission_mode(mut self, mode: PermissionMode) -> Self {
        self.config.permission_mode = mode;
        self
    }

    /// Request structured (typed) output from the agent.
    ///
    /// The type `T` must implement [`JsonSchema`]. The generated schema is sent
    /// to the provider so the model returns JSON conforming to `T`, which can
    /// then be deserialized with [`AgentResult::json`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::prelude::*;
    ///
    /// #[derive(Deserialize, JsonSchema)]
    /// struct Review {
    ///     score: u8,
    ///     summary: String,
    /// }
    ///
    /// # async fn example() -> Result<(), OperationError> {
    /// let provider = ClaudeCodeProvider::new();
    /// let result = Agent::new()
    ///     .prompt("Review the codebase")
    ///     .output::<Review>()
    ///     .run(&provider)
    ///     .await?;
    ///
    /// let review: Review = result.json().expect("schema-validated output");
    /// println!("Score: {}/10 - {}", review.score, review.summary);
    /// # Ok(())
    /// # }
    /// ```
    pub fn output<T: JsonSchema>(mut self) -> Self {
        let schema = schema_for!(T);
        self.config.json_schema = match to_string(&schema) {
            Ok(s) => Some(s),
            Err(e) => {
                warn!(error = %e, type_name = std::any::type_name::<T>(), "failed to serialize JSON schema, structured output disabled");
                None
            }
        };
        self
    }

    /// Enable or disable dry-run mode for this specific operation.
    ///
    /// When dry-run is active, the agent call is logged but not executed.
    /// A synthetic [`AgentResult`] is returned with a placeholder text,
    /// zero cost, and zero tokens.
    ///
    /// If not set, falls back to the global dry-run setting
    /// (see [`set_dry_run`](crate::dry_run::set_dry_run)).
    pub fn dry_run(mut self, enabled: bool) -> Self {
        self.dry_run = Some(enabled);
        self
    }

    /// Resume a previous agent conversation by session ID.
    ///
    /// Pass the session ID from a previous [`AgentResult::session_id()`] to
    /// continue the multi-turn conversation.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::prelude::*;
    ///
    /// # async fn example() -> Result<(), OperationError> {
    /// let provider = ClaudeCodeProvider::new();
    ///
    /// let first = Agent::new()
    ///     .prompt("Analyze the src/ directory")
    ///     .max_budget_usd(0.10)
    ///     .run(&provider)
    ///     .await?;
    ///
    /// let session = first.session_id().expect("provider returned session ID");
    ///
    /// let followup = Agent::new()
    ///     .prompt("Now suggest improvements")
    ///     .resume(session)
    ///     .max_budget_usd(0.10)
    ///     .run(&provider)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if `session_id` is empty or contains characters other than
    /// alphanumerics, hyphens, and underscores.
    pub fn resume(mut self, session_id: &str) -> Self {
        assert!(!session_id.is_empty(), "session_id must not be empty");
        assert!(
            session_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "session_id must only contain alphanumeric characters, hyphens, or underscores, got: {session_id}"
        );
        self.config.resume_session_id = Some(session_id.to_string());
        self
    }

    /// Execute the agent invocation using the given [`AgentProvider`].
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Agent`] if the provider reports a failure
    /// (process crash, timeout, or schema validation error).
    ///
    /// # Panics
    ///
    /// Panics if [`prompt`](Agent::prompt) was never called or the prompt is
    /// empty (whitespace-only counts as empty).
    #[tracing::instrument(name = "agent", skip_all, fields(model = %self.config.model, prompt_len = self.config.prompt.len()))]
    pub async fn run(self, provider: &dyn AgentProvider) -> Result<AgentResult, OperationError> {
        assert!(
            !self.config.prompt.trim().is_empty(),
            "prompt must not be empty - call .prompt(\"...\") before .run()"
        );
        if crate::dry_run::effective_dry_run(self.dry_run) {
            info!(
                prompt_len = self.config.prompt.len(),
                "[dry-run] agent call skipped"
            );
            let mut output =
                AgentOutput::new(Value::String("[dry-run] agent call skipped".to_string()));
            output.cost_usd = Some(0.0);
            output.input_tokens = Some(0);
            output.output_tokens = Some(0);
            return Ok(AgentResult { output });
        }

        let config = self.config;

        #[cfg(feature = "prometheus")]
        let model_label = config.model.to_string();

        let output = match provider.invoke(&config).await {
            Ok(output) => output,
            Err(e) => {
                #[cfg(feature = "prometheus")]
                {
                    metrics::counter!(metric_names::AGENT_TOTAL, "model" => model_label.clone(), "status" => metric_names::STATUS_ERROR).increment(1);
                }
                return Err(OperationError::Agent(e));
            }
        };

        info!(
            duration_ms = output.duration_ms,
            cost_usd = output.cost_usd,
            input_tokens = output.input_tokens,
            output_tokens = output.output_tokens,
            model = output.model,
            "agent completed"
        );

        #[cfg(feature = "prometheus")]
        {
            metrics::counter!(metric_names::AGENT_TOTAL, "model" => model_label.clone(), "status" => metric_names::STATUS_SUCCESS).increment(1);
            metrics::histogram!(metric_names::AGENT_DURATION_SECONDS, "model" => model_label.clone())
                .record(output.duration_ms as f64 / 1000.0);
            if let Some(cost) = output.cost_usd {
                metrics::gauge!(metric_names::AGENT_COST_USD_TOTAL, "model" => model_label.clone())
                    .increment(cost);
            }
            if let Some(tokens) = output.input_tokens {
                metrics::counter!(metric_names::AGENT_TOKENS_INPUT_TOTAL, "model" => model_label.clone()).increment(tokens);
            }
            if let Some(tokens) = output.output_tokens {
                metrics::counter!(metric_names::AGENT_TOKENS_OUTPUT_TOTAL, "model" => model_label)
                    .increment(tokens);
            }
        }

        Ok(AgentResult { output })
    }
}

impl Default for Agent {
    fn default() -> Self {
        Self::new()
    }
}

/// The result of a successful agent invocation.
///
/// Wraps the raw [`AgentOutput`] and provides convenience accessors for the
/// response text, typed JSON deserialization, session metadata, and usage stats.
#[derive(Debug)]
pub struct AgentResult {
    output: AgentOutput,
}

impl AgentResult {
    /// Return the agent's response as a plain text string.
    ///
    /// If the underlying value is not a JSON string (e.g. when structured output
    /// was requested), returns an empty string and logs a warning.
    pub fn text(&self) -> &str {
        match self.output.value.as_str() {
            Some(s) => s,
            None => {
                warn!(
                    value_type = self.output.value.to_string(),
                    "agent output is not a string, returning empty"
                );
                ""
            }
        }
    }

    /// Return the raw JSON [`Value`] of the agent's response.
    pub fn value(&self) -> &Value {
        &self.output.value
    }

    /// Deserialize the agent's response into the given type `T`.
    ///
    /// This clones the underlying JSON value. If you no longer need the
    /// `AgentResult` afterwards, use [`into_json`](AgentResult::into_json)
    /// instead to avoid the clone.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Deserialize`] if the JSON value does not match `T`.
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, OperationError> {
        from_value(self.output.value.clone()).map_err(OperationError::deserialize::<T>)
    }

    /// Consume the result and deserialize the response into `T` without cloning.
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::Deserialize`] if the JSON value does not match `T`.
    pub fn into_json<T: DeserializeOwned>(self) -> Result<T, OperationError> {
        from_value(self.output.value).map_err(OperationError::deserialize::<T>)
    }

    /// Build an `AgentResult` from a raw [`AgentOutput`].
    ///
    /// This is available only in test builds to simplify test setup without
    /// going through the full record/replay pipeline.
    #[cfg(test)]
    pub(crate) fn from_output(output: AgentOutput) -> Self {
        Self { output }
    }

    /// Return the provider-assigned session ID, if available.
    pub fn session_id(&self) -> Option<&str> {
        self.output.session_id.as_deref()
    }

    /// Return the cost of this invocation in USD, if reported by the provider.
    pub fn cost_usd(&self) -> Option<f64> {
        self.output.cost_usd
    }

    /// Return the number of input tokens consumed, if reported.
    pub fn input_tokens(&self) -> Option<u64> {
        self.output.input_tokens
    }

    /// Return the number of output tokens generated, if reported.
    pub fn output_tokens(&self) -> Option<u64> {
        self.output.output_tokens
    }

    /// Return the wall-clock duration of the invocation in milliseconds.
    pub fn duration_ms(&self) -> u64 {
        self.output.duration_ms
    }

    /// Return the concrete model identifier used, if reported by the provider.
    pub fn model(&self) -> Option<&str> {
        self.output.model.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::InvokeFuture;
    use serde_json::json;

    struct TestProvider {
        output: AgentOutput,
    }

    impl AgentProvider for TestProvider {
        fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
            Box::pin(async move {
                Ok(AgentOutput {
                    value: self.output.value.clone(),
                    session_id: self.output.session_id.clone(),
                    cost_usd: self.output.cost_usd,
                    input_tokens: self.output.input_tokens,
                    output_tokens: self.output.output_tokens,
                    model: self.output.model.clone(),
                    duration_ms: self.output.duration_ms,
                })
            })
        }
    }

    struct ConfigCapture {
        output: AgentOutput,
    }

    impl AgentProvider for ConfigCapture {
        fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
            let config_json = serde_json::to_value(config).unwrap();
            Box::pin(async move {
                Ok(AgentOutput {
                    value: config_json,
                    session_id: self.output.session_id.clone(),
                    cost_usd: self.output.cost_usd,
                    input_tokens: self.output.input_tokens,
                    output_tokens: self.output.output_tokens,
                    model: self.output.model.clone(),
                    duration_ms: self.output.duration_ms,
                })
            })
        }
    }

    fn default_output() -> AgentOutput {
        AgentOutput {
            value: json!("test output"),
            session_id: Some("sess-123".to_string()),
            cost_usd: Some(0.05),
            input_tokens: Some(100),
            output_tokens: Some(50),
            model: Some("sonnet".to_string()),
            duration_ms: 1500,
        }
    }

    // --- Model Display ---

    #[test]
    fn model_display_sonnet() {
        assert_eq!(Model::Sonnet.to_string(), "sonnet");
    }

    #[test]
    fn model_display_opus() {
        assert_eq!(Model::Opus.to_string(), "opus");
    }

    #[test]
    fn model_display_haiku() {
        assert_eq!(Model::Haiku.to_string(), "haiku");
    }

    // --- Agent::new() defaults via ConfigCapture ---

    #[tokio::test]
    async fn agent_new_default_values() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let result = Agent::new().prompt("hi").run(&provider).await.unwrap();

        let config = result.value();
        assert_eq!(config["system_prompt"], json!(null));
        assert_eq!(config["prompt"], json!("hi"));
        assert_eq!(config["model"], json!("sonnet"));
        assert_eq!(config["allowed_tools"], json!([]));
        assert_eq!(config["max_turns"], json!(null));
        assert_eq!(config["max_budget_usd"], json!(null));
        assert_eq!(config["working_dir"], json!(null));
        assert_eq!(config["mcp_config"], json!(null));
        assert_eq!(config["permission_mode"], json!("Default"));
        assert_eq!(config["json_schema"], json!(null));
    }

    #[tokio::test]
    async fn agent_default_matches_new() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let result_new = Agent::new().prompt("x").run(&provider).await.unwrap();
        let result_default = Agent::default().prompt("x").run(&provider).await.unwrap();

        assert_eq!(result_new.value(), result_default.value());
    }

    // --- Builder methods ---

    #[tokio::test]
    async fn builder_methods_store_values_correctly() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let result = Agent::new()
            .system_prompt("you are a bot")
            .prompt("do something")
            .model(Model::Opus)
            .allowed_tools(&["Read", "Write"])
            .max_turns(5)
            .max_budget_usd(1.5)
            .working_dir("/tmp")
            .mcp_config("{}")
            .permission_mode(PermissionMode::Auto)
            .run(&provider)
            .await
            .unwrap();

        let config = result.value();
        assert_eq!(config["system_prompt"], json!("you are a bot"));
        assert_eq!(config["prompt"], json!("do something"));
        assert_eq!(config["model"], json!("opus"));
        assert_eq!(config["allowed_tools"], json!(["Read", "Write"]));
        assert_eq!(config["max_turns"], json!(5));
        assert_eq!(config["max_budget_usd"], json!(1.5));
        assert_eq!(config["working_dir"], json!("/tmp"));
        assert_eq!(config["mcp_config"], json!("{}"));
        assert_eq!(config["permission_mode"], json!("Auto"));
    }

    // --- Panics ---

    #[test]
    #[should_panic(expected = "max_turns must be greater than 0")]
    fn max_turns_zero_panics() {
        let _ = Agent::new().max_turns(0);
    }

    #[test]
    #[should_panic(expected = "budget must be a positive finite number")]
    fn max_budget_negative_panics() {
        let _ = Agent::new().max_budget_usd(-1.0);
    }

    #[test]
    #[should_panic(expected = "budget must be a positive finite number")]
    fn max_budget_nan_panics() {
        let _ = Agent::new().max_budget_usd(f64::NAN);
    }

    #[test]
    #[should_panic(expected = "budget must be a positive finite number")]
    fn max_budget_infinity_panics() {
        let _ = Agent::new().max_budget_usd(f64::INFINITY);
    }

    // --- AgentResult accessors ---

    #[tokio::test]
    async fn agent_result_text_with_string_value() {
        let provider = TestProvider {
            output: AgentOutput {
                value: json!("hello world"),
                ..default_output()
            },
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        assert_eq!(result.text(), "hello world");
    }

    #[tokio::test]
    async fn agent_result_text_with_non_string_value() {
        let provider = TestProvider {
            output: AgentOutput {
                value: json!(42),
                ..default_output()
            },
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        assert_eq!(result.text(), "");
    }

    #[tokio::test]
    async fn agent_result_text_with_null_value() {
        let provider = TestProvider {
            output: AgentOutput {
                value: json!(null),
                ..default_output()
            },
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        assert_eq!(result.text(), "");
    }

    #[tokio::test]
    async fn agent_result_json_successful_deserialize() {
        #[derive(Deserialize, PartialEq, Debug)]
        struct MyOutput {
            name: String,
            count: u32,
        }
        let provider = TestProvider {
            output: AgentOutput {
                value: json!({"name": "test", "count": 7}),
                ..default_output()
            },
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        let parsed: MyOutput = result.json().unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.count, 7);
    }

    #[tokio::test]
    async fn agent_result_json_failed_deserialize() {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct MyOutput {
            name: String,
        }
        let provider = TestProvider {
            output: AgentOutput {
                value: json!(42),
                ..default_output()
            },
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        let err = result.json::<MyOutput>().unwrap_err();
        assert!(matches!(err, OperationError::Deserialize { .. }));
    }

    #[tokio::test]
    async fn agent_result_accessors() {
        let provider = TestProvider {
            output: AgentOutput {
                value: json!("v"),
                session_id: Some("s-1".to_string()),
                cost_usd: Some(0.123),
                input_tokens: Some(999),
                output_tokens: Some(456),
                model: Some("opus".to_string()),
                duration_ms: 2000,
            },
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        assert_eq!(result.session_id(), Some("s-1"));
        assert_eq!(result.cost_usd(), Some(0.123));
        assert_eq!(result.input_tokens(), Some(999));
        assert_eq!(result.output_tokens(), Some(456));
        assert_eq!(result.duration_ms(), 2000);
        assert_eq!(result.model(), Some("opus"));
    }

    // --- Session resume ---

    #[tokio::test]
    async fn resume_passes_session_id_in_config() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let result = Agent::new()
            .prompt("followup")
            .resume("sess-abc")
            .run(&provider)
            .await
            .unwrap();

        let config = result.value();
        assert_eq!(config["resume_session_id"], json!("sess-abc"));
    }

    #[tokio::test]
    async fn no_resume_has_null_session_id() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let result = Agent::new()
            .prompt("first call")
            .run(&provider)
            .await
            .unwrap();

        let config = result.value();
        assert_eq!(config["resume_session_id"], json!(null));
    }

    #[test]
    #[should_panic(expected = "session_id must not be empty")]
    fn resume_empty_session_id_panics() {
        let _ = Agent::new().resume("");
    }

    #[test]
    #[should_panic(expected = "session_id must only contain")]
    fn resume_invalid_chars_panics() {
        let _ = Agent::new().resume("sess;rm -rf /");
    }

    #[test]
    fn resume_valid_formats_accepted() {
        let _ = Agent::new().resume("sess-abc123");
        let _ = Agent::new().resume("a1b2c3d4_session");
        let _ = Agent::new().resume("abc-DEF-123_456");
    }

    #[tokio::test]
    #[should_panic(expected = "prompt must not be empty")]
    async fn run_without_prompt_panics() {
        let provider = TestProvider {
            output: default_output(),
        };
        let _ = Agent::new().run(&provider).await;
    }

    #[tokio::test]
    #[should_panic(expected = "prompt must not be empty")]
    async fn run_with_whitespace_only_prompt_panics() {
        let provider = TestProvider {
            output: default_output(),
        };
        let _ = Agent::new().prompt("   ").run(&provider).await;
    }

    // --- Serde roundtrips ---

    #[test]
    fn model_serialize_deserialize_roundtrip() {
        for model in [
            Model::Sonnet,
            Model::Opus,
            Model::Haiku,
            Model::Haiku45,
            Model::Sonnet46,
            Model::Opus46,
            Model::Sonnet46_1M,
            Model::Opus46_1M,
        ] {
            let json = serde_json::to_string(&model).unwrap();
            let back: Model = serde_json::from_str(&json).unwrap();
            assert_eq!(model.to_string(), back.to_string());
        }
    }

    #[test]
    fn model_display_explicit_ids() {
        assert_eq!(Model::Haiku45.to_string(), "claude-haiku-4-5-20251001");
        assert_eq!(Model::Sonnet46.to_string(), "claude-sonnet-4-6");
        assert_eq!(Model::Opus46.to_string(), "claude-opus-4-6");
        assert_eq!(Model::Sonnet46_1M.to_string(), "claude-sonnet-4-6[1m]");
        assert_eq!(Model::Opus46_1M.to_string(), "claude-opus-4-6[1m]");
    }

    #[test]
    fn model_context_window() {
        assert_eq!(Model::Sonnet.context_window(), 200_000);
        assert_eq!(Model::Opus.context_window(), 200_000);
        assert_eq!(Model::Haiku.context_window(), 200_000);
        assert_eq!(Model::Sonnet46_1M.context_window(), 1_000_000);
        assert_eq!(Model::Opus46_1M.context_window(), 1_000_000);
    }

    #[tokio::test]
    async fn into_json_success() {
        #[derive(Deserialize, PartialEq, Debug)]
        struct Out {
            name: String,
        }
        let provider = TestProvider {
            output: AgentOutput {
                value: json!({"name": "test"}),
                ..default_output()
            },
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        let parsed: Out = result.into_json().unwrap();
        assert_eq!(parsed.name, "test");
    }

    #[tokio::test]
    async fn into_json_failure() {
        #[derive(Debug, Deserialize)]
        #[allow(dead_code)]
        struct Out {
            name: String,
        }
        let provider = TestProvider {
            output: AgentOutput {
                value: json!(42),
                ..default_output()
            },
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        let err = result.into_json::<Out>().unwrap_err();
        assert!(matches!(err, OperationError::Deserialize { .. }));
    }

    #[test]
    fn from_output_creates_result() {
        let output = AgentOutput {
            value: json!("hello"),
            ..default_output()
        };
        let result = AgentResult::from_output(output);
        assert_eq!(result.text(), "hello");
        assert_eq!(result.cost_usd(), Some(0.05));
    }

    #[test]
    #[should_panic(expected = "budget must be a positive finite number")]
    fn max_budget_zero_panics() {
        let _ = Agent::new().max_budget_usd(0.0);
    }

    #[test]
    fn model_equality() {
        assert_eq!(Model::Sonnet, Model::Sonnet);
        assert_ne!(Model::Sonnet, Model::Opus);
    }

    #[test]
    fn permission_mode_serialize_deserialize_roundtrip() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::Auto,
            PermissionMode::DontAsk,
            PermissionMode::BypassPermissions,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: PermissionMode = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{:?}", mode), format!("{:?}", back));
        }
    }
}
