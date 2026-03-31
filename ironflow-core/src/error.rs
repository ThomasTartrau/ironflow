//! Error types for ironflow operations.
//!
//! This module defines two error enums:
//!
//! * [`OperationError`] - top-level error returned by both shell and agent operations.
//! * [`AgentError`] - agent-specific error returned by [`AgentProvider::invoke`](crate::provider::AgentProvider::invoke).
//!
//! [`AgentError`] converts into [`OperationError`] via the [`From`] trait, so agent
//! errors propagate naturally through the `?` operator.

use std::time::Duration;

use thiserror::Error;

/// Top-level error for any workflow operation (shell or agent).
///
/// Every public operation in ironflow returns `Result<T, OperationError>`.
#[derive(Debug, Error)]
pub enum OperationError {
    /// A shell command exited with a non-zero status code.
    #[error("shell exited with code {exit_code}: {stderr}")]
    Shell {
        /// Process exit code, or `-1` if the process could not be spawned.
        exit_code: i32,
        /// Captured stderr, truncated to [`MAX_OUTPUT_SIZE`](crate::utils::MAX_OUTPUT_SIZE).
        stderr: String,
    },

    /// An agent invocation failed.
    ///
    /// Wraps an [`AgentError`] with full detail about the failure.
    #[error("agent error: {0}")]
    Agent(#[from] AgentError),

    /// An operation exceeded its configured timeout.
    #[error("step '{step}' timed out after {limit:?}")]
    Timeout {
        /// Human-readable description of the timed-out step (usually the command string).
        step: String,
        /// The [`Duration`] that was exceeded.
        limit: Duration,
    },

    /// An HTTP request failed at the transport layer or the response body
    /// could not be read.
    #[error("{}", match status {
        Some(code) => format!("http error (status {code}): {message}"),
        None => format!("http error: {message}"),
    })]
    Http {
        /// HTTP status code, if a response was received.
        status: Option<u16>,
        /// Human-readable error description.
        message: String,
    },

    /// Failed to deserialize a JSON response into the expected Rust type.
    ///
    /// Returned by [`AgentResult::json`](crate::operations::agent::AgentResult::json)
    /// and [`HttpOutput::json`](crate::operations::http::HttpOutput::json).
    #[error("failed to deserialize into {target_type}: {reason}")]
    Deserialize {
        /// The Rust type name that was expected.
        target_type: String,
        /// The underlying serde error message.
        reason: String,
    },
}

impl OperationError {
    /// Build a [`Deserialize`](OperationError::Deserialize) error for type `T`.
    pub fn deserialize<T>(error: impl std::fmt::Display) -> Self {
        Self::Deserialize {
            target_type: std::any::type_name::<T>().to_string(),
            reason: error.to_string(),
        }
    }
}

/// Error specific to agent (AI provider) invocations.
///
/// Returned by [`AgentProvider::invoke`](crate::provider::AgentProvider::invoke) and
/// automatically wrapped into [`OperationError::Agent`] when propagated with `?`.
#[derive(Debug, Error)]
pub enum AgentError {
    /// The agent process exited with a non-zero status code.
    #[error("claude process exited with code {exit_code}: {stderr}")]
    ProcessFailed {
        /// Process exit code, or `-1` if spawning failed.
        exit_code: i32,
        /// Captured stderr.
        stderr: String,
    },

    /// The agent output did not match the expected schema.
    #[error("schema validation failed: expected {expected}, got {got}")]
    SchemaValidation {
        /// What was expected (e.g. `"structured_output field"`).
        expected: String,
        /// What was actually received.
        got: String,
    },

    /// The prompt exceeds the model's context window.
    ///
    /// Returned before spawning the process when the estimated token count
    /// exceeds the model's known limit.
    ///
    /// * `chars` - number of characters in the combined prompt (system + user).
    /// * `estimated_tokens` - approximate token count (chars / 4).
    /// * `model_limit` - the model's context window in tokens.
    #[error(
        "prompt too large: {chars} chars (~{estimated_tokens} tokens) exceeds model limit of {model_limit} tokens"
    )]
    PromptTooLarge {
        /// Number of characters in the prompt.
        chars: usize,
        /// Estimated token count (chars / 4 heuristic).
        estimated_tokens: usize,
        /// Model's context window in tokens.
        model_limit: usize,
    },

    /// The agent did not complete within the configured timeout.
    #[error("agent timed out after {limit:?}")]
    Timeout {
        /// The [`Duration`] that was exceeded.
        limit: Duration,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_display_format() {
        let err = OperationError::Shell {
            exit_code: 127,
            stderr: "command not found".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "shell exited with code 127: command not found"
        );
    }

    #[test]
    fn agent_display_delegates_to_agent_error() {
        let inner = AgentError::ProcessFailed {
            exit_code: 1,
            stderr: "boom".to_string(),
        };
        let err = OperationError::Agent(inner);
        assert_eq!(
            err.to_string(),
            "agent error: claude process exited with code 1: boom"
        );
    }

    #[test]
    fn timeout_display_format() {
        let err = OperationError::Timeout {
            step: "build".to_string(),
            limit: Duration::from_secs(30),
        };
        assert_eq!(err.to_string(), "step 'build' timed out after 30s");
    }

    #[test]
    fn agent_error_process_failed_display_zero_exit_code() {
        let err = AgentError::ProcessFailed {
            exit_code: 0,
            stderr: "unexpected".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "claude process exited with code 0: unexpected"
        );
    }

    #[test]
    fn agent_error_process_failed_display_negative_exit_code() {
        let err = AgentError::ProcessFailed {
            exit_code: -1,
            stderr: "killed".to_string(),
        };
        assert!(err.to_string().contains("-1"));
    }

    #[test]
    fn agent_error_schema_validation_display() {
        let err = AgentError::SchemaValidation {
            expected: "object".to_string(),
            got: "string".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "schema validation failed: expected object, got string"
        );
    }

    #[test]
    fn agent_error_timeout_display() {
        let err = AgentError::Timeout {
            limit: Duration::from_secs(300),
        };
        assert_eq!(err.to_string(), "agent timed out after 300s");
    }

    #[test]
    fn from_agent_error_process_failed() {
        let agent_err = AgentError::ProcessFailed {
            exit_code: 42,
            stderr: "fail".to_string(),
        };
        let op_err: OperationError = agent_err.into();
        assert!(matches!(
            op_err,
            OperationError::Agent(AgentError::ProcessFailed { exit_code: 42, .. })
        ));
    }

    #[test]
    fn from_agent_error_schema_validation() {
        let agent_err = AgentError::SchemaValidation {
            expected: "a".to_string(),
            got: "b".to_string(),
        };
        let op_err: OperationError = agent_err.into();
        assert!(matches!(
            op_err,
            OperationError::Agent(AgentError::SchemaValidation { .. })
        ));
    }

    #[test]
    fn from_agent_error_timeout() {
        let agent_err = AgentError::Timeout {
            limit: Duration::from_secs(60),
        };
        let op_err: OperationError = agent_err.into();
        assert!(matches!(
            op_err,
            OperationError::Agent(AgentError::Timeout { .. })
        ));
    }

    #[test]
    fn operation_error_implements_std_error() {
        let err = OperationError::Shell {
            exit_code: 1,
            stderr: "x".to_string(),
        };
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn agent_error_implements_std_error() {
        let err = AgentError::Timeout {
            limit: Duration::from_secs(60),
        };
        let _: &dyn std::error::Error = &err;
    }

    #[test]
    fn empty_stderr_edge_case() {
        let err = OperationError::Shell {
            exit_code: 1,
            stderr: String::new(),
        };
        assert_eq!(err.to_string(), "shell exited with code 1: ");
    }

    #[test]
    fn multiline_stderr() {
        let err = AgentError::ProcessFailed {
            exit_code: 1,
            stderr: "line1\nline2\nline3".to_string(),
        };
        assert!(err.to_string().contains("line1\nline2\nline3"));
    }

    #[test]
    fn unicode_in_stderr() {
        let err = OperationError::Shell {
            exit_code: 1,
            stderr: "erreur: fichier introuvable \u{1F4A5}".to_string(),
        };
        assert!(err.to_string().contains("\u{1F4A5}"));
    }

    #[test]
    fn http_error_with_status_display() {
        let err = OperationError::Http {
            status: Some(500),
            message: "internal server error".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "http error (status 500): internal server error"
        );
    }

    #[test]
    fn http_error_without_status_display() {
        let err = OperationError::Http {
            status: None,
            message: "connection refused".to_string(),
        };
        assert_eq!(err.to_string(), "http error: connection refused");
    }

    #[test]
    fn http_error_empty_message() {
        let err = OperationError::Http {
            status: Some(404),
            message: String::new(),
        };
        assert_eq!(err.to_string(), "http error (status 404): ");
    }

    #[test]
    fn subsecond_duration_in_timeout_display() {
        let err = OperationError::Timeout {
            step: "fast".to_string(),
            limit: Duration::from_millis(500),
        };
        assert_eq!(err.to_string(), "step 'fast' timed out after 500ms");
    }

    #[test]
    fn source_chains_agent_error() {
        use std::error::Error;
        let err = OperationError::Agent(AgentError::Timeout {
            limit: Duration::from_secs(60),
        });
        assert!(err.source().is_some());
    }

    #[test]
    fn source_none_for_shell() {
        use std::error::Error;
        let err = OperationError::Shell {
            exit_code: 1,
            stderr: "x".to_string(),
        };
        assert!(err.source().is_none());
    }

    #[test]
    fn deserialize_helper_formats_correctly() {
        let err = OperationError::deserialize::<Vec<String>>(format_args!("missing field"));
        match &err {
            OperationError::Deserialize {
                target_type,
                reason,
            } => {
                assert!(target_type.contains("Vec"));
                assert!(target_type.contains("String"));
                assert_eq!(reason, "missing field");
            }
            _ => panic!("expected Deserialize variant"),
        }
    }

    #[test]
    fn deserialize_display_format() {
        let err = OperationError::Deserialize {
            target_type: "MyStruct".to_string(),
            reason: "bad input".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "failed to deserialize into MyStruct: bad input"
        );
    }

    #[test]
    fn agent_error_prompt_too_large_display() {
        let err = AgentError::PromptTooLarge {
            chars: 966_007,
            estimated_tokens: 241_501,
            model_limit: 200_000,
        };
        let msg = err.to_string();
        assert!(msg.contains("966007 chars"));
        assert!(msg.contains("241501 tokens"));
        assert!(msg.contains("200000 tokens"));
    }

    #[test]
    fn from_agent_error_prompt_too_large() {
        let agent_err = AgentError::PromptTooLarge {
            chars: 1_000_000,
            estimated_tokens: 250_000,
            model_limit: 200_000,
        };
        let op_err: OperationError = agent_err.into();
        assert!(matches!(
            op_err,
            OperationError::Agent(AgentError::PromptTooLarge {
                model_limit: 200_000,
                ..
            })
        ));
    }

    #[test]
    fn source_none_for_http_timeout_deserialize() {
        use std::error::Error;
        let http = OperationError::Http {
            status: Some(500),
            message: "x".to_string(),
        };
        assert!(http.source().is_none());

        let timeout = OperationError::Timeout {
            step: "x".to_string(),
            limit: Duration::from_secs(1),
        };
        assert!(timeout.source().is_none());

        let deser = OperationError::Deserialize {
            target_type: "T".to_string(),
            reason: "r".to_string(),
        };
        assert!(deser.source().is_none());
    }
}
