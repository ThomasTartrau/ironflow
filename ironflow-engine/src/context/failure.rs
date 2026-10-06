//! Helpers shared by the step lifecycle and the parallel wave when a step fails.
//!
//! They decide whether a failure is worth retrying, turn an `allow_failure`
//! error into a usable [`StepOutput`], and pull the partial usage an agent
//! reported before it failed so the run totals stay accurate.

use rust_decimal::Decimal;
use serde_json::{Value, json, to_value};

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::retry::is_retryable;

use crate::error::EngineError;
use crate::executor::{ERROR_KEY, StepArtifacts, StepOutput};

#[cfg(feature = "prometheus")]
pub(super) fn record_retry_metric(kind: &str, outcome: &str) {
    use ironflow_core::metric_names::STEP_RETRIES_TOTAL;
    use metrics::counter;
    counter!(STEP_RETRIES_TOTAL, "kind" => kind.to_string(), "outcome" => outcome.to_string())
        .increment(1);
}

#[cfg(not(feature = "prometheus"))]
pub(super) fn record_retry_metric(_kind: &str, _outcome: &str) {}

/// Step-level retryability: broader than operation-level retry because the user
/// explicitly opted in. Excludes only deterministic or financially wasteful
/// errors that retrying cannot fix.
pub(super) fn is_step_retryable(err: &EngineError) -> bool {
    match err {
        EngineError::Operation(op) => match op {
            OperationError::Agent(AgentError::PromptTooLarge { .. }) => false,
            OperationError::Agent(AgentError::BudgetExceeded { .. }) => false,
            OperationError::Agent(
                AgentError::UnknownToolProfile { .. } | AgentError::ToolProfileUnsupported { .. },
            ) => false,
            // Account selection already decided: replaying hits the same limits.
            OperationError::Agent(
                AgentError::NoCapacity { .. }
                | AgentError::CapacityWait { .. }
                | AgentError::AccountNotFound { .. },
            ) => false,
            // A 4xx from the model API fails the same way on every attempt.
            OperationError::Agent(AgentError::Api { .. }) => is_retryable(op),
            OperationError::Deserialize { .. } => false,
            OperationError::Http {
                status: Some(code), ..
            } if (400..500).contains(code) && *code != 429 => false,
            _ => true,
        },
        _ => false,
    }
}

pub(super) fn allowed_failure_output(
    error_msg: &str,
    raw_response: Option<Value>,
    partial: Option<&StepPartialUsage>,
) -> StepOutput {
    StepOutput {
        output: raw_response.unwrap_or_else(|| json!({ (ERROR_KEY): error_msg })),
        duration_ms: partial.and_then(|p| p.duration_ms).unwrap_or(0),
        cost_usd: partial.and_then(|p| p.cost_usd).unwrap_or(Decimal::ZERO),
        input_tokens: partial.and_then(|p| p.input_tokens),
        cache_read_input_tokens: partial.and_then(|p| p.cache_read_input_tokens),
        cache_creation_input_tokens: partial.and_then(|p| p.cache_creation_input_tokens),
        output_tokens: partial.and_then(|p| p.output_tokens),
        model: None,
        debug_messages: None,
        artifacts: StepArtifacts::default(),
        account_id: None,
        environment_id: None,
    }
}

/// Extract debug messages from an engine error, if it wraps a schema validation
/// failure that carries a verbose conversation trace.
pub(super) fn extract_debug_messages_from_error(err: &EngineError) -> Option<Value> {
    if let EngineError::Operation(OperationError::Agent(AgentError::SchemaValidation {
        debug_messages,
        ..
    })) = err
        && !debug_messages.is_empty()
    {
        return to_value(debug_messages).ok();
    }
    None
}

/// Partial usage with `Decimal` cost, converted from the `f64` in [`PartialUsage`].
///
/// Exists only because `ironflow-store` uses [`Decimal`] for monetary values
/// while `ironflow-core` uses `f64` (the CLI's native type). The conversion
/// happens here, at the engine/store boundary.
pub(super) struct StepPartialUsage {
    pub(super) cost_usd: Option<Decimal>,
    pub(super) duration_ms: Option<u64>,
    pub(super) input_tokens: Option<u64>,
    pub(super) cache_read_input_tokens: Option<u64>,
    pub(super) cache_creation_input_tokens: Option<u64>,
    pub(super) output_tokens: Option<u64>,
}

/// Extract the raw response text from a schema validation error.
///
/// When the agent produced text but structured output extraction failed,
/// this returns the truncated raw text so it can be persisted as the
/// step output for dashboard visibility.
pub(super) fn extract_raw_response_from_error(err: &EngineError) -> Option<Value> {
    if let EngineError::Operation(OperationError::Agent(AgentError::SchemaValidation {
        raw_response: Some(text),
        ..
    })) = err
    {
        return Some(Value::String(text.clone()));
    }
    None
}

pub(super) fn extract_partial_usage_from_error(err: &EngineError) -> Option<StepPartialUsage> {
    if let EngineError::Operation(OperationError::Agent(AgentError::SchemaValidation {
        partial_usage,
        ..
    })) = err
        && (partial_usage.cost_usd.is_some() || partial_usage.duration_ms.is_some())
    {
        return Some(StepPartialUsage {
            cost_usd: partial_usage
                .cost_usd
                .and_then(|c| Decimal::try_from(c).ok()),
            duration_ms: partial_usage.duration_ms,
            input_tokens: partial_usage.input_tokens,
            cache_read_input_tokens: partial_usage.cache_read_input_tokens,
            cache_creation_input_tokens: partial_usage.cache_creation_input_tokens,
            output_tokens: partial_usage.output_tokens,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::Utc;

    use super::*;

    fn agent_error(err: AgentError) -> EngineError {
        EngineError::Operation(OperationError::Agent(err))
    }

    #[test]
    fn tool_profile_errors_are_not_step_retryable() {
        assert!(!is_step_retryable(&agent_error(
            AgentError::UnknownToolProfile {
                profile: "bgu".to_string(),
                available: vec!["bug".to_string()],
            }
        )));
        assert!(!is_step_retryable(&agent_error(
            AgentError::ToolProfileUnsupported {
                provider: "claude-code".to_string(),
                profile: "bug".to_string(),
            }
        )));
        // A transient failure stays retryable: the arm above is not a catch-all.
        assert!(is_step_retryable(&agent_error(AgentError::Timeout {
            limit: Duration::from_secs(1),
        })));
    }

    #[test]
    fn capacity_errors_are_not_step_retryable() {
        assert!(!is_step_retryable(&agent_error(AgentError::NoCapacity {
            kind: "claude".to_string(),
            next_reset: None,
        })));
        assert!(!is_step_retryable(&agent_error(AgentError::CapacityWait {
            kind: "claude".to_string(),
            wake_at: Utc::now(),
        })));
        assert!(!is_step_retryable(&agent_error(
            AgentError::AccountNotFound {
                name: "team-a".to_string(),
            }
        )));
    }

    fn api_error(status: Option<u16>, code: Option<&str>) -> EngineError {
        agent_error(AgentError::Api {
            status,
            code: code.map(str::to_string),
            message: "API Error".to_string(),
        })
    }

    #[test]
    fn api_error_4xx_is_not_step_retryable() {
        assert!(!is_step_retryable(&api_error(
            Some(400),
            Some("claude_code_version_too_old")
        )));
        assert!(!is_step_retryable(&api_error(Some(404), None)));
    }

    #[test]
    fn api_error_transient_is_step_retryable() {
        assert!(is_step_retryable(&api_error(Some(529), None)));
        assert!(is_step_retryable(&api_error(Some(429), None)));
        assert!(is_step_retryable(&api_error(None, None)));
    }
}
