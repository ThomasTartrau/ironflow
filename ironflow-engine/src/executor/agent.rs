//! Agent step executor.

use std::sync::Arc;
use std::time::Instant;

use rust_decimal::Decimal;
use tracing::{info, warn};

use ironflow_core::operations::agent::Agent;
use ironflow_core::provider::{AgentConfig, AgentProvider};

use crate::error::EngineError;
use crate::log_sender::StepLogSender;
use crate::notify::LogStream;

use super::{StepExecutor, StepOutput};

/// Executor for agent (AI) steps.
///
/// Runs an AI agent with the given prompt and configuration, capturing
/// the response value, cost, and token counts. When a [`StepLogSender`]
/// is attached, emits system log lines for step start/end.
pub struct AgentExecutor<'a> {
    config: &'a AgentConfig,
    log_sender: Option<StepLogSender>,
}

impl<'a> AgentExecutor<'a> {
    /// Create a new agent executor from a config reference.
    pub fn new(config: &'a AgentConfig) -> Self {
        Self {
            config,
            log_sender: None,
        }
    }

    /// Attach a log sender for system-level log lines.
    pub fn with_log_sender(mut self, sender: StepLogSender) -> Self {
        self.log_sender = Some(sender);
        self
    }
}

impl StepExecutor for AgentExecutor<'_> {
    async fn execute(&self, provider: &Arc<dyn AgentProvider>) -> Result<StepOutput, EngineError> {
        let start = Instant::now();

        if let Some(ref sender) = self.log_sender {
            sender.emit(
                LogStream::System,
                &format!("agent step started (model={})", self.config.model),
            );
        }

        if self.config.json_schema.is_some() && self.config.max_turns == Some(1) {
            warn!(
                "structured output (json_schema) requires max_turns >= 2; \
                 max_turns is set to 1, the agent will likely fail with error_max_turns"
            );
        }

        let result = Agent::from_config(self.config.clone())
            .run(provider.as_ref())
            .await?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let cost = Decimal::try_from(result.cost_usd().unwrap_or(0.0)).unwrap_or(Decimal::ZERO);
        let input_tokens = result.input_tokens();
        let output_tokens = result.output_tokens();

        info!(
            step_kind = "agent",
            model = %self.config.model,
            cost_usd = %cost,
            input_tokens = ?input_tokens,
            output_tokens = ?output_tokens,
            duration_ms,
            "agent step completed"
        );

        #[cfg(feature = "prometheus")]
        {
            use ironflow_core::metric_names::{
                AGENT_COST_USD_TOTAL, AGENT_DURATION_SECONDS, AGENT_TOKENS_INPUT_TOTAL,
                AGENT_TOKENS_OUTPUT_TOTAL, AGENT_TOTAL, STATUS_SUCCESS,
            };
            use metrics::{counter, gauge, histogram};
            let model_label = self.config.model.clone();
            counter!(AGENT_TOTAL, "model" => model_label.clone(), "status" => STATUS_SUCCESS)
                .increment(1);
            histogram!(AGENT_DURATION_SECONDS, "model" => model_label.clone())
                .record(duration_ms as f64 / 1000.0);
            gauge!(AGENT_COST_USD_TOTAL, "model" => model_label.clone())
                .increment(cost.to_string().parse::<f64>().unwrap_or(0.0));
            if let Some(inp) = input_tokens {
                counter!(AGENT_TOKENS_INPUT_TOTAL, "model" => model_label.clone()).increment(inp);
            }
            if let Some(out) = output_tokens {
                counter!(AGENT_TOKENS_OUTPUT_TOTAL, "model" => model_label).increment(out);
            }
        }

        if let Some(ref sender) = self.log_sender {
            sender.emit(
                LogStream::System,
                &format!(
                    "agent step completed (cost=${cost}, tokens_in={}, tokens_out={})",
                    input_tokens.unwrap_or(0),
                    output_tokens.unwrap_or(0),
                ),
            );
        }

        let debug_messages = result.debug_messages().map(|msgs| msgs.to_vec());

        Ok(StepOutput {
            output: result.value().clone(),
            duration_ms,
            cost_usd: cost,
            input_tokens,
            output_tokens,
            model: result.model().map(String::from),
            debug_messages,
        })
    }
}

#[cfg(test)]
mod tests {
    use ironflow_core::operations::agent::PermissionMode;

    #[test]
    fn parse_permission_mode_via_serde() {
        let json = r#""auto""#;
        let mode: PermissionMode = serde_json::from_str(json).unwrap();
        assert!(matches!(mode, PermissionMode::Auto));
    }

    #[test]
    fn parse_permission_mode_dont_ask() {
        let json = r#""dont_ask""#;
        let mode: PermissionMode = serde_json::from_str(json).unwrap();
        assert!(matches!(mode, PermissionMode::DontAsk));
    }

    #[test]
    fn parse_permission_mode_bypass() {
        let json = r#""bypass""#;
        let mode: PermissionMode = serde_json::from_str(json).unwrap();
        assert!(matches!(mode, PermissionMode::BypassPermissions));
    }

    #[test]
    fn parse_permission_mode_case_insensitive() {
        let json = r#""AUTO""#;
        let mode: PermissionMode = serde_json::from_str(json).unwrap();
        assert!(matches!(mode, PermissionMode::Auto));

        let json = r#""DONT_ASK""#;
        let mode: PermissionMode = serde_json::from_str(json).unwrap();
        assert!(matches!(mode, PermissionMode::DontAsk));
    }

    #[test]
    fn parse_permission_mode_unknown_defaults() {
        let json = r#""unknown""#;
        let mode: PermissionMode = serde_json::from_str(json).unwrap();
        assert!(matches!(mode, PermissionMode::Default));
    }
}
