//! Step executor — reconstructs operations from configs and runs them.
//!
//! Each step type (shell, HTTP, agent) has its own executor implementing
//! the [`StepExecutor`] trait. The [`execute_step_config`] function dispatches
//! to the appropriate executor based on the [`StepConfig`] variant.

mod agent;
mod http;
mod shell;

use std::future::Future;
use std::sync::Arc;

use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;

use ironflow_core::provider::{AgentProvider, DebugMessage};

use crate::config::StepConfig;
use crate::error::EngineError;
use crate::log_sender::StepLogSender;

pub use agent::AgentExecutor;
pub use http::HttpExecutor;
pub use shell::ShellExecutor;

/// Result of executing a single step.
#[derive(Debug, Clone)]
pub struct StepOutput {
    /// Serialized output (stdout for shell, body for http, value for agent).
    ///
    /// For agent steps with a JSON schema, the value may not strictly conform
    /// to the schema: Claude CLI can flatten wrapper objects with a single
    /// array field, returning a bare array instead of `{"items": [...]}`.
    /// Callers should handle both the expected wrapper and a bare value.
    pub output: Value,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Cost in USD (agent steps only).
    pub cost_usd: Decimal,
    /// Input token count (agent steps only).
    pub input_tokens: Option<u64>,
    /// Output token count (agent steps only).
    pub output_tokens: Option<u64>,
    /// Model identifier used for agent steps (e.g. `"claude-sonnet-4-20250514"`).
    pub model: Option<String>,
    /// Conversation trace from verbose agent invocations.
    pub debug_messages: Option<Vec<DebugMessage>>,
}

impl StepOutput {
    /// Serialize debug messages to a JSON [`Value`] for store persistence.
    ///
    /// Returns `None` when verbose mode was off (no messages captured).
    pub fn debug_messages_json(&self) -> Option<Value> {
        self.debug_messages
            .as_ref()
            .and_then(|msgs| serde_json::to_value(msgs).ok())
    }
}

/// Result of a single step within a [`parallel`](crate::context::WorkflowContext::parallel) batch.
#[derive(Debug, Clone)]
pub struct ParallelStepResult {
    /// The step name (same as provided to `parallel()`).
    pub name: String,
    /// The step execution output.
    pub output: StepOutput,
    /// The step ID in the store (for dependency tracking).
    pub step_id: Uuid,
}

/// Trait for step executors.
///
/// Each step type implements this trait to execute its specific operation
/// and return a [`StepOutput`].
pub trait StepExecutor: Send + Sync {
    /// Execute the step and return structured output.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError`] if the operation fails.
    fn execute(
        &self,
        provider: &Arc<dyn AgentProvider>,
    ) -> impl Future<Output = Result<StepOutput, EngineError>> + Send;
}

/// Execute a [`StepConfig`] and return structured output.
///
/// When a [`StepLogSender`] is provided, executors that support streaming
/// will emit log lines in real time (e.g. shell stdout/stderr).
///
/// # Errors
///
/// Returns [`EngineError::Operation`] if the operation fails.
///
/// # Examples
///
/// ```no_run
/// use ironflow_engine::config::{StepConfig, ShellConfig};
/// use ironflow_engine::executor::execute_step_config;
/// use ironflow_core::provider::AgentProvider;
/// use ironflow_core::providers::claude::ClaudeCodeProvider;
/// use std::sync::Arc;
///
/// # async fn example() -> Result<(), ironflow_engine::error::EngineError> {
/// let provider: Arc<dyn AgentProvider> = Arc::new(ClaudeCodeProvider::new());
/// let config = StepConfig::Shell(ShellConfig::new("echo hello"));
/// let output = execute_step_config(&config, &provider, None).await?;
/// # Ok(())
/// # }
/// ```
pub async fn execute_step_config(
    config: &StepConfig,
    provider: &Arc<dyn AgentProvider>,
    log_sender: Option<StepLogSender>,
) -> Result<StepOutput, EngineError> {
    let _kind = match config {
        StepConfig::Shell(_) => "shell",
        StepConfig::Http(_) => "http",
        StepConfig::Agent(_) => "agent",
        StepConfig::Workflow(_) => "workflow",
        StepConfig::Approval(_) => "approval",
    };

    let result = match config {
        StepConfig::Shell(cfg) => {
            let mut executor = ShellExecutor::new(cfg);
            if let Some(sender) = log_sender {
                executor = executor.with_log_sender(sender);
            }
            executor.execute(provider).await
        }
        StepConfig::Http(cfg) => HttpExecutor::new(cfg).execute(provider).await,
        StepConfig::Agent(cfg) => {
            let mut executor = AgentExecutor::new(cfg);
            if let Some(sender) = log_sender {
                executor = executor.with_log_sender(sender);
            }
            executor.execute(provider).await
        }
        StepConfig::Workflow(_) => Err(EngineError::StepConfig(
            "workflow steps are executed by WorkflowContext, not the executor".to_string(),
        )),
        StepConfig::Approval(_) => Err(EngineError::StepConfig(
            "approval steps are executed by WorkflowContext, not the executor".to_string(),
        )),
    };

    #[cfg(feature = "prometheus")]
    {
        use ironflow_core::metric_names::{
            STATUS_ERROR, STATUS_SUCCESS, STEP_DURATION_SECONDS, STEPS_TOTAL,
        };
        use metrics::{counter, histogram};
        let status = if result.is_ok() {
            STATUS_SUCCESS
        } else {
            STATUS_ERROR
        };
        counter!(STEPS_TOTAL, "kind" => _kind, "status" => status).increment(1);
        if let Ok(ref output) = result {
            histogram!(STEP_DURATION_SECONDS, "kind" => _kind)
                .record(output.duration_ms as f64 / 1000.0);
        }
    }

    result
}
