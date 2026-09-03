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
use serde::de::DeserializeOwned;
use serde_json::{Value, from_value};
use uuid::Uuid;

use ironflow_core::provider::{AgentProvider, DebugMessage};
use ironflow_store::entities::StepStatus;

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

    /// Exit code of a shell step.
    ///
    /// Returns `None` for non-shell steps or when the field is absent.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::StepOutput;
    /// use rust_decimal::Decimal;
    /// use serde_json::json;
    ///
    /// let output = StepOutput {
    ///     output: json!({"stdout": "ok\n", "stderr": "", "exit_code": 0}),
    ///     duration_ms: 3,
    ///     cost_usd: Decimal::ZERO,
    ///     input_tokens: None,
    ///     output_tokens: None,
    ///     model: None,
    ///     debug_messages: None,
    /// };
    /// assert_eq!(output.exit_code(), Some(0));
    /// ```
    pub fn exit_code(&self) -> Option<i64> {
        self.output.get("exit_code").and_then(Value::as_i64)
    }

    /// Standard output of a shell step, or an empty string for other kinds.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::StepOutput;
    /// use rust_decimal::Decimal;
    /// use serde_json::json;
    ///
    /// let output = StepOutput {
    ///     output: json!({"stdout": "42 tests passed\n", "stderr": "", "exit_code": 0}),
    ///     duration_ms: 3,
    ///     cost_usd: Decimal::ZERO,
    ///     input_tokens: None,
    ///     output_tokens: None,
    ///     model: None,
    ///     debug_messages: None,
    /// };
    /// assert!(output.stdout().contains("42 tests"));
    /// ```
    pub fn stdout(&self) -> &str {
        self.output
            .get("stdout")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// Standard error of a shell step, or an empty string for other kinds.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::StepOutput;
    /// use rust_decimal::Decimal;
    /// use serde_json::json;
    ///
    /// let output = StepOutput {
    ///     output: json!({"stdout": "", "stderr": "warning: unused", "exit_code": 0}),
    ///     duration_ms: 3,
    ///     cost_usd: Decimal::ZERO,
    ///     input_tokens: None,
    ///     output_tokens: None,
    ///     model: None,
    ///     debug_messages: None,
    /// };
    /// assert_eq!(output.stderr(), "warning: unused");
    /// ```
    pub fn stderr(&self) -> &str {
        self.output
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// HTTP status code of an HTTP step.
    ///
    /// Returns `None` for non-HTTP steps or when the field is absent.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::StepOutput;
    /// use rust_decimal::Decimal;
    /// use serde_json::json;
    ///
    /// let output = StepOutput {
    ///     output: json!({"status": 204, "body": ""}),
    ///     duration_ms: 3,
    ///     cost_usd: Decimal::ZERO,
    ///     input_tokens: None,
    ///     output_tokens: None,
    ///     model: None,
    ///     debug_messages: None,
    /// };
    /// assert_eq!(output.status(), Some(204));
    /// ```
    pub fn status(&self) -> Option<u16> {
        self.output
            .get("status")
            .and_then(Value::as_u64)
            .and_then(|s| u16::try_from(s).ok())
    }

    /// Response body of an HTTP step, or an empty string for other kinds.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::StepOutput;
    /// use rust_decimal::Decimal;
    /// use serde_json::json;
    ///
    /// let output = StepOutput {
    ///     output: json!({"status": 200, "body": "{\"ok\":true}"}),
    ///     duration_ms: 3,
    ///     cost_usd: Decimal::ZERO,
    ///     input_tokens: None,
    ///     output_tokens: None,
    ///     model: None,
    ///     debug_messages: None,
    /// };
    /// assert_eq!(output.body(), "{\"ok\":true}");
    /// ```
    pub fn body(&self) -> &str {
        self.output
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    /// Whether the step succeeded from the point of view of its own kind.
    ///
    /// - Shell step: the exit code is `0`.
    /// - HTTP step: the status is in the `2xx` range.
    /// - Any other kind: `false`, since no success marker is recorded.
    ///
    /// Mostly useful after a step configured with `allow_failure()`, since a
    /// failing step otherwise returns an error from the context method.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::StepOutput;
    /// use rust_decimal::Decimal;
    /// use serde_json::json;
    ///
    /// let shell = StepOutput {
    ///     output: json!({"stdout": "", "stderr": "", "exit_code": 1}),
    ///     duration_ms: 3,
    ///     cost_usd: Decimal::ZERO,
    ///     input_tokens: None,
    ///     output_tokens: None,
    ///     model: None,
    ///     debug_messages: None,
    /// };
    /// assert!(!shell.is_success());
    ///
    /// let http = StepOutput { output: json!({"status": 201, "body": ""}), ..shell.clone() };
    /// assert!(http.is_success());
    /// ```
    pub fn is_success(&self) -> bool {
        if let Some(code) = self.exit_code() {
            return code == 0;
        }
        if let Some(status) = self.status() {
            return (200..300).contains(&status);
        }
        false
    }

    /// Deserialize the step output into `T`.
    ///
    /// Intended for agent steps constrained by a JSON schema, and for custom
    /// operations that return structured JSON.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Serialization`] when the output does not match `T`.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::executor::StepOutput;
    /// use rust_decimal::Decimal;
    /// use serde::Deserialize;
    /// use serde_json::json;
    ///
    /// #[derive(Deserialize)]
    /// struct Review {
    ///     score: u8,
    /// }
    ///
    /// let output = StepOutput {
    ///     output: json!({"score": 8}),
    ///     duration_ms: 3,
    ///     cost_usd: Decimal::ZERO,
    ///     input_tokens: None,
    ///     output_tokens: None,
    ///     model: None,
    ///     debug_messages: None,
    /// };
    /// let review: Review = output.json()?;
    /// assert_eq!(review.score, 8);
    /// # Ok::<(), ironflow_engine::error::EngineError>(())
    /// ```
    pub fn json<T: DeserializeOwned>(&self) -> Result<T, EngineError> {
        from_value(self.output.clone()).map_err(EngineError::Serialization)
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

/// Enriched result of a completed step, for post-execution inspection.
///
/// Collects the step's trace ID, status, metrics, and a truncated output
/// summary into a single struct that the [`WorkflowContext`](crate::context::WorkflowContext)
/// accumulates over the run.
///
/// # Examples
///
/// ```
/// use ironflow_engine::executor::StepResult;
/// use ironflow_store::entities::StepStatus;
/// use rust_decimal::Decimal;
/// use uuid::Uuid;
///
/// let result = StepResult {
///     trace_id: Uuid::nil(),
///     name: "build".to_string(),
///     status: StepStatus::Completed,
///     duration_ms: 1200,
///     cost_usd: Decimal::ZERO,
///     input_tokens: None,
///     output_tokens: None,
///     error: None,
///     output_summary: Some("ok".to_string()),
/// };
/// assert_eq!(result.status, StepStatus::Completed);
/// ```
#[derive(Debug, Clone, serde::Serialize)]
pub struct StepResult {
    /// Deterministic trace ID for log correlation.
    pub trace_id: Uuid,
    /// Step name.
    pub name: String,
    /// Terminal status.
    pub status: StepStatus,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Cost in USD.
    pub cost_usd: Decimal,
    /// Input token count (agent steps only).
    pub input_tokens: Option<u64>,
    /// Output token count (agent steps only).
    pub output_tokens: Option<u64>,
    /// Error message if the step failed.
    pub error: Option<String>,
    /// First 500 characters of the serialized output.
    pub output_summary: Option<String>,
}

/// Maximum length of [`StepResult::output_summary`].
const OUTPUT_SUMMARY_MAX_LEN: usize = 500;

impl StepResult {
    /// Build from a completed step's output.
    pub fn from_success(trace_id: Uuid, name: &str, output: &StepOutput) -> Self {
        Self {
            trace_id,
            name: name.to_string(),
            status: StepStatus::Completed,
            duration_ms: output.duration_ms,
            cost_usd: output.cost_usd,
            input_tokens: output.input_tokens,
            output_tokens: output.output_tokens,
            error: None,
            output_summary: summarize_output(&output.output),
        }
    }

    /// Build from a failed step.
    pub fn from_failure(
        trace_id: Uuid,
        name: &str,
        error: &str,
        duration_ms: u64,
        cost_usd: Decimal,
    ) -> Self {
        Self {
            trace_id,
            name: name.to_string(),
            status: StepStatus::Failed,
            duration_ms,
            cost_usd,
            input_tokens: None,
            output_tokens: None,
            error: Some(error.to_string()),
            output_summary: None,
        }
    }
}

fn summarize_output(value: &Value) -> Option<String> {
    let raw = value.to_string();
    match raw.char_indices().nth(OUTPUT_SUMMARY_MAX_LEN) {
        None => Some(raw),
        Some((byte_idx, _)) => Some(raw[..byte_idx].to_string()),
    }
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
#[tracing::instrument(name = "executor.execute_step", skip_all, fields(step.kind))]
pub async fn execute_step_config(
    config: &StepConfig,
    provider: &Arc<dyn AgentProvider>,
    log_sender: Option<StepLogSender>,
) -> Result<StepOutput, EngineError> {
    let kind = match config {
        StepConfig::Shell(_) => "shell",
        StepConfig::Http(_) => "http",
        StepConfig::Agent(_) => "agent",
        StepConfig::Workflow(_) => "workflow",
        StepConfig::Approval(_) => "approval",
    };
    tracing::Span::current().record("step.kind", kind);

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
        counter!(STEPS_TOTAL, "kind" => kind, "status" => status).increment(1);
        if let Ok(ref output) = result {
            histogram!(STEP_DURATION_SECONDS, "kind" => kind)
                .record(output.duration_ms as f64 / 1000.0);
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironflow_core::provider::DebugMessage;
    use serde_json::json;

    #[test]
    fn step_output_with_no_debug_messages_returns_none() {
        let output = StepOutput {
            output: json!({"result": "ok"}),
            duration_ms: 100,
            cost_usd: rust_decimal::Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
        };

        assert_eq!(output.debug_messages_json(), None);
    }

    #[test]
    fn step_output_with_empty_debug_messages_returns_some_empty_array() {
        let output = StepOutput {
            output: json!({"result": "ok"}),
            duration_ms: 100,
            cost_usd: rust_decimal::Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: Some(Vec::new()),
        };

        let json_val = output.debug_messages_json();
        assert!(json_val.is_some());
        let arr = json_val.unwrap();
        assert!(arr.is_array());
        assert_eq!(arr.as_array().unwrap().len(), 0);
    }

    #[test]
    fn step_output_debug_messages_json_serializes_messages() {
        let json_msgs = json!([
            {
                "text": "Hello",
                "thinking": null,
                "thinking_redacted": false,
                "tool_calls": [],
                "tool_results": [],
                "stop_reason": "end_turn",
                "input_tokens": 10,
                "output_tokens": 20
            },
            {
                "text": "Hi there",
                "thinking": null,
                "thinking_redacted": false,
                "tool_calls": [],
                "tool_results": [],
                "stop_reason": "end_turn",
                "input_tokens": 15,
                "output_tokens": 25
            }
        ]);

        let messages: Vec<DebugMessage> =
            serde_json::from_value(json_msgs.clone()).expect("deserialize debug messages");

        let output = StepOutput {
            output: json!({"result": "ok"}),
            duration_ms: 100,
            cost_usd: rust_decimal::Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: Some(messages),
        };

        let json_val = output.debug_messages_json();
        assert!(json_val.is_some());

        let arr = json_val.unwrap();
        assert!(arr.is_array());
        let messages_array = arr.as_array().unwrap();
        assert_eq!(messages_array.len(), 2);
        assert_eq!(messages_array[0]["text"], "Hello");
        assert_eq!(messages_array[1]["text"], "Hi there");
    }

    #[test]
    fn step_output_contains_all_metrics() {
        let output = StepOutput {
            output: json!({"data": "test"}),
            duration_ms: 5000,
            cost_usd: rust_decimal::Decimal::new(123, 2),
            input_tokens: Some(100),
            output_tokens: Some(200),
            model: Some("claude-sonnet".to_string()),
            debug_messages: None,
        };

        assert_eq!(output.duration_ms, 5000);
        assert_eq!(output.cost_usd, rust_decimal::Decimal::new(123, 2));
        assert_eq!(output.input_tokens, Some(100));
        assert_eq!(output.output_tokens, Some(200));
        assert_eq!(output.model, Some("claude-sonnet".to_string()));
    }

    #[test]
    fn step_output_default_tokens_and_model_are_none() {
        let output = StepOutput {
            output: json!({}),
            duration_ms: 0,
            cost_usd: rust_decimal::Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
        };

        assert!(output.input_tokens.is_none());
        assert!(output.output_tokens.is_none());
        assert!(output.model.is_none());
    }

    #[test]
    fn parallel_step_result_contains_step_metadata() {
        let step_id = uuid::Uuid::now_v7();
        let output = StepOutput {
            output: json!({"done": true}),
            duration_ms: 1000,
            cost_usd: rust_decimal::Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
        };

        let result = ParallelStepResult {
            name: "build".to_string(),
            output,
            step_id,
        };

        assert_eq!(result.name, "build");
        assert_eq!(result.step_id, step_id);
        assert_eq!(result.output.duration_ms, 1000);
    }

    #[test]
    fn step_output_serializes_complex_json_output() {
        let complex_output = json!({
            "status": "success",
            "data": {
                "items": [1, 2, 3],
                "nested": {
                    "key": "value"
                }
            }
        });

        let output = StepOutput {
            output: complex_output.clone(),
            duration_ms: 100,
            cost_usd: rust_decimal::Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
        };

        assert_eq!(output.output, complex_output);
        assert_eq!(output.output["status"], "success");
        assert_eq!(output.output["data"]["items"][0], 1);
        assert_eq!(output.output["data"]["nested"]["key"], "value");
    }

    #[test]
    fn step_result_from_success_captures_all_fields() {
        let trace_id = Uuid::nil();
        let output = StepOutput {
            output: json!({"stdout": "ok"}),
            duration_ms: 1500,
            cost_usd: Decimal::new(42, 2),
            input_tokens: Some(100),
            output_tokens: Some(200),
            model: Some("claude-sonnet".to_string()),
            debug_messages: None,
        };

        let result = StepResult::from_success(trace_id, "build", &output);

        assert_eq!(result.trace_id, trace_id);
        assert_eq!(result.name, "build");
        assert_eq!(result.status, StepStatus::Completed);
        assert_eq!(result.duration_ms, 1500);
        assert_eq!(result.cost_usd, Decimal::new(42, 2));
        assert_eq!(result.input_tokens, Some(100));
        assert_eq!(result.output_tokens, Some(200));
        assert!(result.error.is_none());
        assert!(result.output_summary.is_some());
        assert!(result.output_summary.unwrap().contains("stdout"));
    }

    #[test]
    fn step_result_from_failure_captures_error() {
        let trace_id = Uuid::nil();
        let result =
            StepResult::from_failure(trace_id, "deploy", "connection refused", 500, Decimal::ZERO);

        assert_eq!(result.trace_id, trace_id);
        assert_eq!(result.name, "deploy");
        assert_eq!(result.status, StepStatus::Failed);
        assert_eq!(result.duration_ms, 500);
        assert_eq!(result.error, Some("connection refused".to_string()));
        assert!(result.output_summary.is_none());
    }

    #[test]
    fn step_result_output_summary_truncates_long_output() {
        let long_value = json!({"data": "x".repeat(1000)});
        let output = StepOutput {
            output: long_value,
            duration_ms: 0,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
        };

        let result = StepResult::from_success(Uuid::nil(), "test", &output);
        let summary = result.output_summary.unwrap();
        assert_eq!(summary.len(), 500);
    }
}

#[cfg(test)]
mod output_helper_tests {
    use super::*;
    use serde::Deserialize;
    use serde_json::json;

    fn output(value: Value) -> StepOutput {
        StepOutput {
            output: value,
            duration_ms: 1,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
        }
    }

    #[test]
    fn shell_helpers_read_shell_fields() {
        let out = output(json!({"stdout": "hi\n", "stderr": "warn", "exit_code": 0}));
        assert_eq!(out.exit_code(), Some(0));
        assert_eq!(out.stdout(), "hi\n");
        assert_eq!(out.stderr(), "warn");
        assert!(out.is_success());
        assert_eq!(out.status(), None);
        assert_eq!(out.body(), "");
    }

    #[test]
    fn shell_non_zero_exit_is_not_success() {
        let out = output(json!({"stdout": "", "stderr": "", "exit_code": 127}));
        assert_eq!(out.exit_code(), Some(127));
        assert!(!out.is_success());
    }

    #[test]
    fn http_helpers_read_http_fields() {
        let out = output(json!({"status": 200, "body": "{\"ok\":true}"}));
        assert_eq!(out.status(), Some(200));
        assert_eq!(out.body(), "{\"ok\":true}");
        assert!(out.is_success());
        assert_eq!(out.exit_code(), None);
        assert_eq!(out.stdout(), "");
    }

    #[test]
    fn http_error_status_is_not_success() {
        assert!(!output(json!({"status": 500, "body": ""})).is_success());
        assert!(!output(json!({"status": 199, "body": ""})).is_success());
        assert!(output(json!({"status": 299, "body": ""})).is_success());
    }

    #[test]
    fn status_out_of_u16_range_is_none() {
        assert_eq!(output(json!({"status": 70000})).status(), None);
        assert_eq!(output(json!({"status": "200"})).status(), None);
    }

    #[test]
    fn agent_output_without_markers_is_not_success() {
        let out = output(json!({"summary": "fine"}));
        assert!(!out.is_success());
        assert_eq!(out.exit_code(), None);
        assert_eq!(out.stdout(), "");
        assert_eq!(out.body(), "");
    }

    #[test]
    fn json_deserializes_structured_output() {
        #[derive(Deserialize, Debug, PartialEq)]
        struct Review {
            score: u8,
            summary: String,
        }
        let out = output(json!({"score": 9, "summary": "good"}));
        let review: Review = out.json().expect("matches schema");
        assert_eq!(
            review,
            Review {
                score: 9,
                summary: "good".to_string()
            }
        );
    }

    #[test]
    fn json_reports_mismatch_as_serialization_error() {
        #[derive(Deserialize, Debug)]
        struct Review {
            #[allow(dead_code)]
            score: u8,
        }
        let out = output(json!({"score": "nine"}));
        let err = out.json::<Review>().expect_err("type mismatch");
        assert!(matches!(err, EngineError::Serialization(_)));
    }

    #[test]
    fn helpers_tolerate_non_object_output() {
        let out = output(json!("plain text"));
        assert_eq!(out.exit_code(), None);
        assert_eq!(out.status(), None);
        assert_eq!(out.stdout(), "");
        assert!(!out.is_success());
    }
}
