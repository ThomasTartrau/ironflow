//! Helpers shared by the step lifecycle and the parallel wave when a step fails.
//!
//! They decide whether a failure is worth retrying, turn an `allow_failure`
//! error into a usable [`StepOutput`], and pull the partial usage an agent
//! reported before it failed so the run totals stay accurate.

use rust_decimal::Decimal;
use serde_json::{Value, json, to_value};

use ironflow_core::error::{AgentError, OperationError};

use crate::error::EngineError;
use crate::executor::StepOutput;

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
        output: raw_response.unwrap_or_else(|| json!({"error": error_msg})),
        duration_ms: partial.and_then(|p| p.duration_ms).unwrap_or(0),
        cost_usd: partial.and_then(|p| p.cost_usd).unwrap_or(Decimal::ZERO),
        input_tokens: partial.and_then(|p| p.input_tokens),
        output_tokens: partial.and_then(|p| p.output_tokens),
        model: None,
        debug_messages: None,
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
            output_tokens: partial_usage.output_tokens,
        });
    }
    None
}
