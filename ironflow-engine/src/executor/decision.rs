//! Decision step executor.
//!
//! Runs a [`DecisionConfig`](crate::config::DecisionConfig) against a
//! [`DecisionProvider`](ironflow_core::decision::DecisionProvider) and records the
//! USD cost reported by [`DecisionUsage::cost_usd`](ironflow_core::decision::DecisionUsage::cost_usd).

use std::sync::Arc;
use std::time::Instant;

use ironflow_core::decision::{DecisionOutput, DecisionProvider};
use ironflow_core::error::OperationError;
use rust_decimal::Decimal;

use crate::config::DecisionConfig;
use crate::error::EngineError;

/// The result of executing a decision step: the typed output plus accounting.
#[derive(Debug, Clone)]
pub struct DecisionExecution {
    /// The typed answers returned by the provider.
    pub output: DecisionOutput,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: u64,
    /// Cost in USD, imputed from input tokens at the Jev rate.
    pub cost_usd: Decimal,
    /// Input token count.
    pub input_tokens: u64,
    /// Output token count (typically zero for System One models).
    pub output_tokens: u64,
}

/// Execute a decision request against the provider.
///
/// # Errors
///
/// Returns [`EngineError::Operation`] wrapping the provider error if the backend
/// fails, times out, or returns an unparseable response.
pub async fn execute_decision(
    provider: &Arc<dyn DecisionProvider>,
    config: &DecisionConfig,
) -> Result<DecisionExecution, EngineError> {
    let request = config.to_request();
    let start = Instant::now();
    let output = provider
        .decide(&request)
        .await
        .map_err(OperationError::from)?;
    let duration_ms = start.elapsed().as_millis() as u64;
    let input_tokens = output.usage.input_tokens;
    let output_tokens = output.usage.output_tokens;
    Ok(DecisionExecution {
        cost_usd: output.usage.cost_usd(),
        output,
        duration_ms,
        input_tokens,
        output_tokens,
    })
}
