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
//!     .model(Model::SONNET)
//!     .max_turns(3)
//!     .run(&provider)
//!     .await?;
//!
//! println!("{}", result.text());
//! # Ok(())
//! # }
//! ```

use std::any;
use std::sync::Arc;

use schemars::{JsonSchema, schema_for};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, from_value, to_string};
use tokio::time;
use tracing::{info, warn};

use crate::error::OperationError;
#[cfg(feature = "prometheus")]
use crate::metric_names;
use crate::provider::{AgentConfig, AgentOutput, AgentProvider, DebugMessage, LogSink};
use crate::retry::RetryPolicy;
use crate::trace_context::WorkflowTraceContext;

/// Provider-agnostic model identifiers.
///
/// Constants are provided for well-known Claude models, but any string
/// is accepted - custom [`AgentProvider`] implementations interpret the
/// model identifier however they wish.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::prelude::*;
///
/// # async fn example() -> Result<(), OperationError> {
/// let provider = ClaudeCodeProvider::new();
///
/// // Using a built-in constant
/// let r = Agent::new()
///     .prompt("hi")
///     .model(Model::SONNET)
///     .run(&provider)
///     .await?;
///
/// // Using a custom model string
/// let r = Agent::new()
///     .prompt("hi")
///     .model("mistral-large-latest")
///     .run(&provider)
///     .await?;
/// # Ok(())
/// # }
/// ```
pub struct Model;

impl Model {
    // ── Aliases (latest version, CLI resolves to current) ───────────

    /// Claude Sonnet - balanced speed and capability (default).
    pub const SONNET: &str = "sonnet";
    /// Claude Opus - highest capability.
    pub const OPUS: &str = "opus";
    /// Claude Haiku - fastest and cheapest.
    pub const HAIKU: &str = "haiku";

    // ── Claude 4.5 ─────────────────────────────────────────────────

    /// Claude Haiku 4.5.
    pub const HAIKU_45: &str = "claude-haiku-4-5-20251001";

    // ── Claude 4.6 - 200K context ──────────────────────────────────

    /// Claude Sonnet 4.6.
    pub const SONNET_46: &str = "claude-sonnet-4-6";
    /// Claude Opus 4.6.
    pub const OPUS_46: &str = "claude-opus-4-6";

    // ── Claude 4.6 - 1M context ────────────────────────────────────

    /// Claude Sonnet 4.6 with 1M token context window.
    pub const SONNET_46_1M: &str = "claude-sonnet-4-6[1m]";
    /// Claude Opus 4.6 with 1M token context window.
    pub const OPUS_46_1M: &str = "claude-opus-4-6[1m]";

    // ── Claude 4.7 - 1M context native ─────────────────────────────

    /// Claude Opus 4.7 - previous flagship, 1M token context native.
    pub const OPUS_47: &str = "claude-opus-4-7";
    /// Claude Opus 4.7 with 1M token context window explicit.
    pub const OPUS_47_1M: &str = "claude-opus-4-7[1m]";

    // ── Claude 4.8 - 1M context native ─────────────────────────────

    /// Claude Opus 4.8 - previous Opus flagship, 1M token context native.
    pub const OPUS_48: &str = "claude-opus-4-8";
    /// Claude Opus 4.8 with 1M token context window explicit.
    pub const OPUS_48_1M: &str = "claude-opus-4-8[1m]";

    // ── Claude 5 - 1M context native ───────────────────────────────

    /// Claude Fable 5 - most capable widely released model, 1M token context native.
    pub const FABLE_5: &str = "claude-fable-5";
    /// Claude Fable 5.1 - latest Fable iteration, improved reasoning, 1M token context native.
    pub const FABLE_51: &str = "claude-fable-5-1";
    /// Claude Mythos 5 - Fable 5 capabilities, limited availability (Project Glasswing).
    pub const MYTHOS_5: &str = "claude-mythos-5";
    /// Claude Opus 5 - current flagship for agentic coding, 1M token context native.
    pub const OPUS_5: &str = "claude-opus-5";
    /// Claude Opus 5 with 1M token context window explicit.
    pub const OPUS_5_1M: &str = "claude-opus-5[1m]";
    /// Claude Sonnet 5 - best speed/intelligence balance, 1M token context native.
    pub const SONNET_5: &str = "claude-sonnet-5";
    /// Claude Sonnet 5 with 1M token context window explicit.
    pub const SONNET_5_1M: &str = "claude-sonnet-5[1m]";
}

/// Controls how the agent handles tool-use permission prompts.
///
/// These map to the `--permission-mode` and `--dangerously-skip-permissions`
/// flags in the Claude CLI.
#[derive(Debug, Default, Clone, Copy, Serialize)]
pub enum PermissionMode {
    /// Use the CLI default permission behavior.
    #[default]
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

impl<'de> Deserialize<'de> for PermissionMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(match s.to_lowercase().replace('_', "").as_str() {
            "auto" => Self::Auto,
            "dontask" => Self::DontAsk,
            "bypass" | "bypasspermissions" => Self::BypassPermissions,
            _ => Self::Default,
        })
    }
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
///     .model(Model::OPUS)
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
    retry_policy: Option<RetryPolicy>,
    log_sink: Option<Arc<dyn LogSink>>,
}

impl Agent {
    /// Create a new agent builder with default settings.
    ///
    /// Defaults: [`Model::SONNET`], no system prompt, no tool restrictions,
    /// no budget/turn limits, [`PermissionMode::Default`].
    pub fn new() -> Self {
        Self {
            config: AgentConfig::new(""),
            dry_run: None,
            retry_policy: None,
            log_sink: None,
        }
    }

    /// Create an agent builder from an existing [`AgentConfig`].
    ///
    /// Useful when the config comes from a serialized workflow definition
    /// rather than being built programmatically.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::prelude::*;
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// # async fn example() -> Result<(), OperationError> {
    /// let provider = ClaudeCodeProvider::new();
    /// let config = AgentConfig::new("Summarize the README");
    /// let result = Agent::from_config(config).run(&provider).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn from_config(config: impl Into<AgentConfig>) -> Self {
        Self {
            config: config.into(),
            dry_run: None,
            retry_policy: None,
            log_sink: None,
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

    /// Set the model to use for this invocation.
    ///
    /// Accepts any string-like value. Use [`Model`] constants for well-known
    /// Claude models, or pass an arbitrary string for custom providers.
    ///
    /// Defaults to [`Model::SONNET`] if not called.
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.config.model = model.into();
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
                warn!(error = %e, type_name = any::type_name::<T>(), "failed to serialize JSON schema, structured output disabled");
                None
            }
        };
        self
    }

    /// Set structured output from a pre-serialized JSON Schema string.
    ///
    /// Use this when the schema comes from configuration or another source
    /// rather than a Rust type. For type-safe schema generation, prefer
    /// [`output`](Agent::output).
    ///
    /// **Important:** structured output requires `max_turns >= 2`. The Claude CLI
    /// uses the first turn for reasoning and a second turn to produce the
    /// schema-conforming JSON.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::prelude::*;
    ///
    /// # async fn example() -> Result<(), OperationError> {
    /// let schema = r#"{"type":"object","properties":{"labels":{"type":"array","items":{"type":"string"}}}}"#;
    /// let agent = Agent::new()
    ///     .prompt("Classify this email")
    ///     .output_schema_raw(schema);
    /// # Ok(())
    /// # }
    /// ```
    pub fn output_schema_raw(mut self, schema: &str) -> Self {
        self.config.json_schema = Some(schema.to_string());
        self
    }

    /// Retry the agent invocation up to `max_retries` times on transient failures.
    ///
    /// Uses default exponential backoff settings (200ms initial, 2x multiplier,
    /// 30s cap). For custom backoff parameters, use [`retry_policy`](Agent::retry_policy).
    ///
    /// Only transient errors are retried: process failures and timeouts.
    /// Deterministic errors (prompt too large, schema validation) are never retried.
    ///
    /// # Panics
    ///
    /// Panics if `max_retries` is `0`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::prelude::*;
    ///
    /// # async fn example() -> Result<(), OperationError> {
    /// let provider = ClaudeCodeProvider::new();
    /// let result = Agent::new()
    ///     .prompt("Summarize the codebase")
    ///     .retry(2)
    ///     .run(&provider)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn retry(mut self, max_retries: u32) -> Self {
        self.retry_policy = Some(RetryPolicy::new(max_retries));
        self
    }

    /// Set a custom [`RetryPolicy`] for this agent invocation.
    ///
    /// Allows full control over backoff duration, multiplier, and max delay.
    /// See [`RetryPolicy`] for details.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_core::prelude::*;
    /// use ironflow_core::retry::RetryPolicy;
    ///
    /// # async fn example() -> Result<(), OperationError> {
    /// let provider = ClaudeCodeProvider::new();
    /// let result = Agent::new()
    ///     .prompt("Analyze the code")
    ///     .retry_policy(
    ///         RetryPolicy::new(3)
    ///             .backoff(Duration::from_secs(1))
    ///             .max_backoff(Duration::from_secs(60))
    ///     )
    ///     .run(&provider)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry_policy = Some(policy);
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

    /// Attach a [`LogSink`] for real-time log streaming.
    ///
    /// When set, [`invoke_with_logs`](AgentProvider::invoke_with_logs) is called
    /// instead of [`invoke`](AgentProvider::invoke), allowing providers that
    /// support streaming to pipe output lines in real time.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::sync::Arc;
    /// use ironflow_core::prelude::*;
    ///
    /// # async fn example() -> Result<(), OperationError> {
    /// # struct MySink;
    /// # impl LogSink for MySink { fn log(&self, _: &str, _: &str) {} }
    /// let provider = ClaudeCodeProvider::new();
    /// let sink: Arc<dyn LogSink> = Arc::new(MySink);
    ///
    /// let result = Agent::new()
    ///     .prompt("Analyze src/")
    ///     .log_sink(sink)
    ///     .run(&provider)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn log_sink(mut self, sink: Arc<dyn LogSink>) -> Self {
        self.log_sink = Some(sink);
        self
    }

    /// Attach a [`WorkflowTraceContext`] for distributed tracing.
    ///
    /// When set, the trace context is stored in the [`AgentConfig`] and
    /// made available to providers for injecting `traceparent` headers
    /// into outgoing HTTP requests.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::prelude::*;
    /// use ironflow_core::trace_context::WorkflowTraceContext;
    ///
    /// # async fn example() -> Result<(), OperationError> {
    /// let provider = ClaudeCodeProvider::new();
    /// let ctx = WorkflowTraceContext::new_root();
    ///
    /// let result = Agent::new()
    ///     .prompt("Analyze the code")
    ///     .trace_context(ctx)
    ///     .run(&provider)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn trace_context(mut self, ctx: WorkflowTraceContext) -> Self {
        self.config.trace_context = Some(ctx);
        self
    }

    /// Enable verbose/debug mode to capture the full conversation trace.
    ///
    /// When enabled, the provider captures every assistant message and tool
    /// call into [`AgentResult::debug_messages`]. Useful for understanding
    /// why the agent returned an unexpected result.
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
    ///     .prompt("Analyze src/")
    ///     .verbose()
    ///     .max_budget_usd(0.10)
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
    pub fn verbose(mut self) -> Self {
        self.config.verbose = true;
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
    /// If a [`retry_policy`](Agent::retry_policy) is configured, transient
    /// failures (process crashes, timeouts, schema validation) are retried
    /// with exponential backoff. When structured output is requested
    /// (`json_schema` is set) and no explicit retry policy is configured,
    /// an automatic retry policy of 2 retries is applied to handle
    /// non-deterministic `structured_output: null` responses from the CLI.
    /// Deterministic errors (prompt too large) are returned immediately
    /// without retry.
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

        let result = self.invoke_once(provider).await;

        let default_schema_retry = RetryPolicy::new(2);
        let policy = match &self.retry_policy {
            Some(p) => p,
            None if self.config.json_schema.is_some() => &default_schema_retry,
            None => return result,
        };

        // Non-retryable errors are returned immediately.
        if let Err(ref err) = result {
            if !crate::retry::is_retryable(err) {
                return result;
            }
        } else {
            return result;
        }

        let mut last_result = result;

        for attempt in 0..policy.max_retries {
            let delay = policy.delay_for_attempt(attempt);
            let retry_reason = if matches!(
                &last_result,
                Err(OperationError::Agent(
                    crate::error::AgentError::SchemaValidation { .. }
                ))
            ) {
                "structured_output was null (CLI non-determinism)"
            } else {
                "transient failure"
            };
            warn!(
                attempt = attempt + 1,
                max_retries = policy.max_retries,
                delay_ms = delay.as_millis() as u64,
                reason = retry_reason,
                "retrying agent invocation"
            );
            time::sleep(delay).await;

            last_result = self.invoke_once(provider).await;

            match &last_result {
                Ok(_) => return last_result,
                Err(err) if !crate::retry::is_retryable(err) => return last_result,
                _ => {}
            }
        }

        last_result
    }

    /// Execute a single agent invocation attempt (no retry logic).
    async fn invoke_once(
        &self,
        provider: &dyn AgentProvider,
    ) -> Result<AgentResult, OperationError> {
        #[cfg(feature = "prometheus")]
        let model_label = self.config.model.to_string();

        let invoke_result = match self.log_sink {
            Some(ref sink) => provider.invoke_with_logs(&self.config, sink.clone()).await,
            None => provider.invoke(&self.config).await,
        };
        let output = match invoke_result {
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

    /// Return the conversation trace captured during a verbose invocation.
    ///
    /// Returns `None` when [`Agent::verbose`] was not called. When present,
    /// each [`DebugMessage`] contains the
    /// assistant's text and tool calls for one conversation turn.
    pub fn debug_messages(&self) -> Option<&[DebugMessage]> {
        self.output.debug_messages.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::AgentError;
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
                    debug_messages: None,
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
                    debug_messages: None,
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
            debug_messages: None,
        }
    }

    // --- Model constants ---

    #[test]
    fn model_constants_have_expected_values() {
        assert_eq!(Model::SONNET, "sonnet");
        assert_eq!(Model::OPUS, "opus");
        assert_eq!(Model::HAIKU, "haiku");
        assert_eq!(Model::HAIKU_45, "claude-haiku-4-5-20251001");
        assert_eq!(Model::SONNET_46, "claude-sonnet-4-6");
        assert_eq!(Model::OPUS_46, "claude-opus-4-6");
        assert_eq!(Model::SONNET_46_1M, "claude-sonnet-4-6[1m]");
        assert_eq!(Model::OPUS_46_1M, "claude-opus-4-6[1m]");
        assert_eq!(Model::OPUS_47, "claude-opus-4-7");
        assert_eq!(Model::OPUS_47_1M, "claude-opus-4-7[1m]");
        assert_eq!(Model::OPUS_48, "claude-opus-4-8");
        assert_eq!(Model::OPUS_48_1M, "claude-opus-4-8[1m]");
        assert_eq!(Model::FABLE_5, "claude-fable-5");
        assert_eq!(Model::FABLE_51, "claude-fable-5-1");
        assert_eq!(Model::MYTHOS_5, "claude-mythos-5");
        assert_eq!(Model::OPUS_5, "claude-opus-5");
        assert_eq!(Model::OPUS_5_1M, "claude-opus-5[1m]");
        assert_eq!(Model::SONNET_5, "claude-sonnet-5");
        assert_eq!(Model::SONNET_5_1M, "claude-sonnet-5[1m]");
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
            .model(Model::OPUS)
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
                debug_messages: None,
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

    // --- Model accepts arbitrary strings ---

    #[tokio::test]
    async fn model_accepts_custom_string() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let result = Agent::new()
            .prompt("hi")
            .model("mistral-large-latest")
            .run(&provider)
            .await
            .unwrap();
        assert_eq!(result.value()["model"], json!("mistral-large-latest"));
    }

    #[tokio::test]
    async fn verbose_sets_config_flag() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let result = Agent::new()
            .prompt("hi")
            .verbose()
            .run(&provider)
            .await
            .unwrap();
        assert_eq!(result.value()["verbose"], json!(true));
    }

    #[tokio::test]
    async fn verbose_not_set_by_default() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let result = Agent::new().prompt("hi").run(&provider).await.unwrap();
        assert_eq!(result.value()["verbose"], json!(false));
    }

    #[tokio::test]
    async fn debug_messages_none_without_verbose() {
        let provider = TestProvider {
            output: default_output(),
        };
        let result = Agent::new().prompt("test").run(&provider).await.unwrap();
        assert!(result.debug_messages().is_none());
    }

    #[tokio::test]
    async fn model_accepts_owned_string() {
        let provider = ConfigCapture {
            output: default_output(),
        };
        let model_name = String::from("gpt-4o");
        let result = Agent::new()
            .prompt("hi")
            .model(model_name)
            .run(&provider)
            .await
            .unwrap();
        assert_eq!(result.value()["model"], json!("gpt-4o"));
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
    fn model_constant_equality() {
        assert_eq!(Model::SONNET, "sonnet");
        assert_ne!(Model::SONNET, Model::OPUS);
    }

    #[test]
    fn permission_mode_serialize_deserialize_roundtrip() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::Auto,
            PermissionMode::DontAsk,
            PermissionMode::BypassPermissions,
        ] {
            let json = to_string(&mode).unwrap();
            let back: PermissionMode = serde_json::from_str(&json).unwrap();
            assert_eq!(format!("{:?}", mode), format!("{:?}", back));
        }
    }

    // --- Retry builder ---

    #[test]
    fn retry_builder_stores_policy() {
        let agent = Agent::new().retry(3);
        assert!(agent.retry_policy.is_some());
        assert_eq!(agent.retry_policy.unwrap().max_retries(), 3);
    }

    #[test]
    fn retry_policy_builder_stores_custom_policy() {
        use crate::retry::RetryPolicy;
        let policy = RetryPolicy::new(5).backoff(Duration::from_secs(1));
        let agent = Agent::new().retry_policy(policy);
        let p = agent.retry_policy.unwrap();
        assert_eq!(p.max_retries(), 5);
    }

    #[test]
    fn no_retry_by_default() {
        let agent = Agent::new();
        assert!(agent.retry_policy.is_none());
    }

    // --- Retry behavior ---

    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    struct FailNTimesProvider {
        fail_count: AtomicU32,
        failures_before_success: u32,
        output: AgentOutput,
    }

    impl AgentProvider for FailNTimesProvider {
        fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
            Box::pin(async move {
                let current = self.fail_count.fetch_add(1, Ordering::SeqCst);
                if current < self.failures_before_success {
                    Err(AgentError::ProcessFailed {
                        exit_code: 1,
                        stderr: format!("transient failure #{}", current + 1),
                    })
                } else {
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
                }
            })
        }
    }

    #[tokio::test]
    async fn retry_succeeds_after_transient_failures() {
        let provider = FailNTimesProvider {
            fail_count: AtomicU32::new(0),
            failures_before_success: 2,
            output: default_output(),
        };
        let result = Agent::new()
            .prompt("test")
            .retry_policy(crate::retry::RetryPolicy::new(3).backoff(Duration::from_millis(1)))
            .run(&provider)
            .await;

        assert!(result.is_ok());
        assert_eq!(provider.fail_count.load(Ordering::SeqCst), 3); // 1 initial + 2 retries
    }

    #[tokio::test]
    async fn retry_exhausted_returns_last_error() {
        let provider = FailNTimesProvider {
            fail_count: AtomicU32::new(0),
            failures_before_success: 10, // always fails
            output: default_output(),
        };
        let result = Agent::new()
            .prompt("test")
            .retry_policy(crate::retry::RetryPolicy::new(2).backoff(Duration::from_millis(1)))
            .run(&provider)
            .await;

        assert!(result.is_err());
        // 1 initial + 2 retries = 3 total
        assert_eq!(provider.fail_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn retry_does_not_retry_prompt_too_large() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        struct CountingNonRetryable {
            count: Arc<AtomicU32>,
        }
        impl AgentProvider for CountingNonRetryable {
            fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
                self.count.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Err(AgentError::PromptTooLarge {
                        chars: 1_000_000,
                        estimated_tokens: 250_000,
                        model_limit: 200_000,
                    })
                })
            }
        }

        let provider = CountingNonRetryable { count };
        let result = Agent::new()
            .prompt("test")
            .retry_policy(crate::retry::RetryPolicy::new(3).backoff(Duration::from_millis(1)))
            .run(&provider)
            .await;

        assert!(result.is_err());
        assert_eq!(call_count.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn retry_retries_schema_validation_errors() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        struct SchemaFailProvider {
            count: Arc<AtomicU32>,
        }
        impl AgentProvider for SchemaFailProvider {
            fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
                self.count.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Err(AgentError::SchemaValidation {
                        expected: "object".to_string(),
                        got: "null".to_string(),
                        debug_messages: Vec::new(),
                        partial_usage: Box::default(),
                        raw_response: None,
                    })
                })
            }
        }

        let provider = SchemaFailProvider { count };
        let result = Agent::new()
            .prompt("test")
            .retry_policy(crate::retry::RetryPolicy::new(2).backoff(Duration::from_millis(1)))
            .run(&provider)
            .await;

        assert!(result.is_err());
        // 1 initial + 2 retries = 3 total
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn schema_validation_succeeds_on_retry() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        struct SchemaFailThenSucceed {
            count: Arc<AtomicU32>,
            output: AgentOutput,
        }
        impl AgentProvider for SchemaFailThenSucceed {
            fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
                let current = self.count.fetch_add(1, Ordering::SeqCst);
                let output = self.output.clone();
                Box::pin(async move {
                    if current == 0 {
                        Err(AgentError::SchemaValidation {
                            expected: "structured_output field".to_string(),
                            got: "null".to_string(),
                            debug_messages: Vec::new(),
                            partial_usage: Box::default(),
                            raw_response: None,
                        })
                    } else {
                        Ok(output)
                    }
                })
            }
        }

        let provider = SchemaFailThenSucceed {
            count,
            output: default_output(),
        };
        let result = Agent::new()
            .prompt("test")
            .retry_policy(crate::retry::RetryPolicy::new(1).backoff(Duration::from_millis(1)))
            .run(&provider)
            .await;

        assert!(result.is_ok());
        assert_eq!(call_count.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn auto_retry_applied_when_json_schema_set() {
        let call_count = Arc::new(AtomicU32::new(0));
        let count = call_count.clone();

        struct AlwaysSchemaFail {
            count: Arc<AtomicU32>,
        }
        impl AgentProvider for AlwaysSchemaFail {
            fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
                self.count.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move {
                    Err(AgentError::SchemaValidation {
                        expected: "object".to_string(),
                        got: "null".to_string(),
                        debug_messages: Vec::new(),
                        partial_usage: Box::default(),
                        raw_response: None,
                    })
                })
            }
        }

        let provider = AlwaysSchemaFail { count };
        let result = Agent::new()
            .prompt("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .run(&provider)
            .await;

        assert!(result.is_err());
        // auto-retry(2) : 1 initial + 2 retries = 3 total
        assert_eq!(call_count.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn no_retry_without_policy() {
        let provider = FailNTimesProvider {
            fail_count: AtomicU32::new(0),
            failures_before_success: 1,
            output: default_output(),
        };
        let result = Agent::new().prompt("test").run(&provider).await;

        assert!(result.is_err());
        assert_eq!(provider.fail_count.load(Ordering::SeqCst), 1);
    }

    // ── log_sink tests ────────────────────────────────────────────

    use crate::test_support::VecSink;

    struct SinkCapture {
        output: AgentOutput,
        saw_logs: Arc<AtomicU32>,
    }

    impl AgentProvider for SinkCapture {
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

        fn invoke_with_logs<'a>(
            &'a self,
            config: &'a AgentConfig,
            log_sink: Arc<dyn LogSink>,
        ) -> InvokeFuture<'a> {
            self.saw_logs.fetch_add(1, Ordering::SeqCst);
            log_sink.log("stdout", "streaming line");
            self.invoke(config)
        }
    }

    #[tokio::test]
    async fn log_sink_routes_to_invoke_with_logs() {
        let saw_logs = Arc::new(AtomicU32::new(0));
        let provider = SinkCapture {
            output: default_output(),
            saw_logs: saw_logs.clone(),
        };
        let sink: Arc<dyn LogSink> = VecSink::new();

        let result = Agent::new()
            .prompt("test")
            .log_sink(sink)
            .run(&provider)
            .await;

        assert!(result.is_ok());
        assert_eq!(saw_logs.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn no_log_sink_routes_to_invoke() {
        let saw_logs = Arc::new(AtomicU32::new(0));
        let provider = SinkCapture {
            output: default_output(),
            saw_logs: saw_logs.clone(),
        };

        let result = Agent::new().prompt("test").run(&provider).await;

        assert!(result.is_ok());
        assert_eq!(saw_logs.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn log_sink_receives_provider_lines() {
        let saw_logs = Arc::new(AtomicU32::new(0));
        let provider = SinkCapture {
            output: default_output(),
            saw_logs: saw_logs.clone(),
        };
        let sink = VecSink::new();

        let _ = Agent::new()
            .prompt("test")
            .log_sink(sink.clone() as Arc<dyn LogSink>)
            .run(&provider)
            .await;

        let lines = sink.0.lock().unwrap();
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].0, "stdout");
        assert_eq!(lines[0].1, "streaming line");
    }
}
