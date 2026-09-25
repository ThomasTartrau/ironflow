//! Agent step executor.

use std::sync::Arc;
use std::time::Instant;

use rust_decimal::Decimal;
use tracing::{info, warn};

use ironflow_core::operations::agent::Agent;
use ironflow_core::pricing::{CostBreakdown, StaticPricing, spawn_log};
use ironflow_core::provider::{AgentConfig, AgentProvider, LogSink};
use ironflow_store::entities::StepKind;

use crate::error::EngineError;
use crate::log_sender::StepLogSender;
use crate::notify::LogStream;

use super::{StepArtifacts, StepExecutor, StepOutput};

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
    fn kind(&self) -> StepKind {
        StepKind::Agent
    }

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

        let mut agent = Agent::from_config(self.config.clone());
        if let Some(ref sender) = self.log_sender {
            agent = agent.log_sink(Arc::new(sender.clone()) as Arc<dyn LogSink>);
        }
        let result = agent.run(provider.as_ref()).await?;

        let duration_ms = start.elapsed().as_millis() as u64;
        let cost = Decimal::try_from(result.cost_usd().unwrap_or(0.0)).unwrap_or(Decimal::ZERO);
        let input_tokens = result.input_tokens();
        let cache_read_tokens = result.cache_read_input_tokens();
        let cache_creation_tokens = result.cache_creation_input_tokens();
        let output_tokens = result.output_tokens();

        info!(
            step_kind = "agent",
            model = %self.config.model,
            cost_usd = %cost,
            input_tokens = ?input_tokens,
            cache_read_input_tokens = ?cache_read_tokens,
            cache_creation_input_tokens = ?cache_creation_tokens,
            output_tokens = ?output_tokens,
            duration_ms,
            "agent step completed"
        );

        let pricing = StaticPricing::new();
        let breakdown = CostBreakdown::compute_with_cache(
            &pricing,
            &self.config.model,
            input_tokens.unwrap_or(0),
            cache_read_tokens.unwrap_or(0),
            cache_creation_tokens.unwrap_or(0),
            output_tokens.unwrap_or(0),
        );
        spawn_log("agent", &self.config.model, breakdown);

        #[cfg(feature = "prometheus")]
        {
            use ironflow_core::metric_names::{
                AGENT_COST_USD_TOTAL, AGENT_DURATION_SECONDS, AGENT_TOKENS_CACHE_READ_TOTAL,
                AGENT_TOKENS_CACHE_WRITE_TOTAL, AGENT_TOKENS_INPUT_TOTAL,
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
            if let Some(t) = cache_read_tokens {
                counter!(AGENT_TOKENS_CACHE_READ_TOTAL, "model" => model_label.clone())
                    .increment(t);
            }
            if let Some(t) = cache_creation_tokens {
                counter!(AGENT_TOKENS_CACHE_WRITE_TOTAL, "model" => model_label.clone())
                    .increment(t);
            }
            if let Some(out) = output_tokens {
                counter!(AGENT_TOKENS_OUTPUT_TOTAL, "model" => model_label).increment(out);
            }
        }

        if let Some(ref sender) = self.log_sender {
            sender.emit(
                LogStream::System,
                &format!(
                    "agent step completed (cost=${cost}, tokens_in={}, cache_read={}, cache_write={}, tokens_out={})",
                    input_tokens.unwrap_or(0),
                    cache_read_tokens.unwrap_or(0),
                    cache_creation_tokens.unwrap_or(0),
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
            cache_read_input_tokens: cache_read_tokens,
            cache_creation_input_tokens: cache_creation_tokens,
            output_tokens,
            model: result.model().map(String::from),
            debug_messages,
            artifacts: StepArtifacts::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use ironflow_core::operations::agent::PermissionMode;
    use ironflow_core::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
    use serde_json::json;
    use tokio::time::timeout;

    use super::{AgentExecutor, StepExecutor};

    /// Real provider returning a fixed output, used to exercise usage propagation.
    struct FixedUsageProvider {
        output: AgentOutput,
    }

    impl AgentProvider for FixedUsageProvider {
        fn invoke<'a>(&'a self, _config: &'a AgentConfig) -> InvokeFuture<'a> {
            Box::pin(async move { Ok(self.output.clone()) })
        }
    }

    fn budget_config() -> AgentConfig {
        let mut config = AgentConfig::new("hi");
        config.max_budget_usd = Some(0.10);
        config
    }

    #[tokio::test]
    async fn agent_executor_propagates_cache_tokens() {
        timeout(Duration::from_secs(10), async {
            let mut output = AgentOutput::new(json!("ok"));
            output.input_tokens = Some(100);
            output.cache_read_input_tokens = Some(5000);
            output.cache_creation_input_tokens = Some(200);
            output.output_tokens = Some(50);
            output.cost_usd = Some(0.02);
            let provider: Arc<dyn AgentProvider> = Arc::new(FixedUsageProvider { output });

            let config = budget_config();
            let step = AgentExecutor::new(&config)
                .execute(&provider)
                .await
                .expect("agent step succeeds");

            assert_eq!(step.input_tokens, Some(100));
            assert_eq!(step.cache_read_input_tokens, Some(5000));
            assert_eq!(step.cache_creation_input_tokens, Some(200));
            assert_eq!(step.output_tokens, Some(50));
            assert_eq!(step.total_tokens(), 5350);
        })
        .await
        .expect("test timed out");
    }

    #[tokio::test]
    async fn agent_executor_without_cache_tokens_yields_none() {
        timeout(Duration::from_secs(10), async {
            let mut output = AgentOutput::new(json!("ok"));
            output.input_tokens = Some(100);
            output.output_tokens = Some(50);
            output.cost_usd = Some(0.02);
            let provider: Arc<dyn AgentProvider> = Arc::new(FixedUsageProvider { output });

            let config = budget_config();
            let step = AgentExecutor::new(&config)
                .execute(&provider)
                .await
                .expect("agent step succeeds");

            assert_eq!(step.input_tokens, Some(100));
            assert!(step.cache_read_input_tokens.is_none());
            assert!(step.cache_creation_input_tokens.is_none());
            assert_eq!(step.total_tokens(), 150);
        })
        .await
        .expect("test timed out");
    }

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
