//! Agent step executor.

use std::sync::Arc;
use std::time::Instant;

use rust_decimal::Decimal;
use serde_json::json;
use tracing::info;

use ironflow_core::operations::agent::{Agent, Model, PermissionMode};
use ironflow_core::provider::AgentProvider;

use crate::config::AgentStepConfig;
use crate::error::EngineError;

use super::{StepExecutor, StepOutput};

/// Executor for agent (AI) steps.
///
/// Runs an AI agent with the given prompt and configuration, capturing
/// the response value, cost, and token counts.
pub struct AgentExecutor<'a> {
    config: &'a AgentStepConfig,
}

impl<'a> AgentExecutor<'a> {
    /// Create a new agent executor from a config reference.
    pub fn new(config: &'a AgentStepConfig) -> Self {
        Self { config }
    }
}

impl StepExecutor for AgentExecutor<'_> {
    async fn execute(&self, provider: &Arc<dyn AgentProvider>) -> Result<StepOutput, EngineError> {
        let start = Instant::now();

        let mut agent = Agent::new().prompt(&self.config.prompt);

        if let Some(ref sp) = self.config.system_prompt {
            agent = agent.system_prompt(sp);
        }
        if let Some(ref model_str) = self.config.model {
            let model = parse_model(model_str)?;
            agent = agent.model(model);
        }
        if let Some(budget) = self.config.max_budget_usd {
            agent = agent.max_budget_usd(budget);
        }
        if let Some(turns) = self.config.max_turns {
            agent = agent.max_turns(turns);
        }
        if !self.config.allowed_tools.is_empty() {
            let tool_refs: Vec<&str> = self
                .config
                .allowed_tools
                .iter()
                .map(|s| s.as_str())
                .collect();
            agent = agent.allowed_tools(&tool_refs);
        }
        if let Some(ref dir) = self.config.working_dir {
            agent = agent.working_dir(dir);
        }
        if let Some(ref mode) = self.config.permission_mode {
            let pm = parse_permission_mode(mode);
            agent = agent.permission_mode(pm);
        }

        let result = agent.run(provider.as_ref()).await?;
        let duration_ms = start.elapsed().as_millis() as u64;
        let cost = Decimal::try_from(result.cost_usd().unwrap_or(0.0)).unwrap_or(Decimal::ZERO);
        let input_tokens = result.input_tokens();
        let output_tokens = result.output_tokens();

        info!(
            step_kind = "agent",
            model = ?self.config.model,
            cost_usd = %cost,
            input_tokens = ?input_tokens,
            output_tokens = ?output_tokens,
            duration_ms,
            "agent step completed"
        );

        Ok(StepOutput {
            output: json!({
                "value": result.value(),
                "model": result.model(),
            }),
            duration_ms,
            cost_usd: cost,
            input_tokens,
            output_tokens,
        })
    }
}

/// Parse a model string into a [`Model`] enum.
///
/// Supports multiple formats for backward compatibility:
/// - "sonnet", "opus", "haiku"
/// - "haiku45", "haiku-4.5"
/// - "sonnet46", "sonnet-4.6"
/// - "opus46", "opus-4.6"
fn parse_model(s: &str) -> Result<Model, EngineError> {
    match s.to_lowercase().as_str() {
        "sonnet" => Ok(Model::Sonnet),
        "opus" => Ok(Model::Opus),
        "haiku" => Ok(Model::Haiku),
        "haiku45" | "haiku-4.5" => Ok(Model::Haiku45),
        "sonnet46" | "sonnet-4.6" => Ok(Model::Sonnet46),
        "opus46" | "opus-4.6" => Ok(Model::Opus46),
        other => Err(EngineError::StepConfig(format!("unknown model: {other}"))),
    }
}

/// Parse a permission mode string into a [`PermissionMode`] enum.
///
/// Unknown values default to [`PermissionMode::Default`].
fn parse_permission_mode(s: &str) -> PermissionMode {
    match s.to_lowercase().as_str() {
        "auto" => PermissionMode::Auto,
        "dont_ask" | "dontask" => PermissionMode::DontAsk,
        "bypass" | "bypass_permissions" => PermissionMode::BypassPermissions,
        _ => PermissionMode::Default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse_model_sonnet() {
        let result = parse_model("sonnet");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Sonnet);
    }

    #[test]
    fn parse_model_opus() {
        let result = parse_model("opus");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Opus);
    }

    #[test]
    fn parse_model_haiku() {
        let result = parse_model("haiku");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Haiku);
    }

    #[test]
    fn parse_model_haiku45() {
        let result = parse_model("haiku45");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Haiku45);
    }

    #[test]
    fn parse_model_haiku_with_dash() {
        let result = parse_model("haiku-4.5");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Haiku45);
    }

    #[test]
    fn parse_model_sonnet46() {
        let result = parse_model("sonnet46");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Sonnet46);
    }

    #[test]
    fn parse_model_sonnet_with_dash() {
        let result = parse_model("sonnet-4.6");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Sonnet46);
    }

    #[test]
    fn parse_model_opus46() {
        let result = parse_model("opus46");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Opus46);
    }

    #[test]
    fn parse_model_opus_with_dash() {
        let result = parse_model("opus-4.6");
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Model::Opus46);
    }

    #[test]
    fn parse_model_unknown_returns_error() {
        let result = parse_model("invalid-model");
        assert!(result.is_err());
        match result {
            Err(EngineError::StepConfig(msg)) => {
                assert!(msg.contains("unknown model"));
            }
            _ => panic!("expected StepConfig error"),
        }
    }

    #[test]
    fn parse_model_case_insensitive() {
        assert!(parse_model("SONNET").is_ok());
        assert!(parse_model("OpUs").is_ok());
        assert!(parse_model("HAIKU").is_ok());
    }

    #[test]
    fn parse_permission_mode_auto() {
        let result = parse_permission_mode("auto");
        assert!(matches!(result, PermissionMode::Auto));
    }

    #[test]
    fn parse_permission_mode_dont_ask() {
        let result = parse_permission_mode("dont_ask");
        assert!(matches!(result, PermissionMode::DontAsk));
    }

    #[test]
    fn parse_permission_mode_dont_ask_alt() {
        let result = parse_permission_mode("dontask");
        assert!(matches!(result, PermissionMode::DontAsk));
    }

    #[test]
    fn parse_permission_mode_bypass() {
        let result = parse_permission_mode("bypass");
        assert!(matches!(result, PermissionMode::BypassPermissions));
    }

    #[test]
    fn parse_permission_mode_bypass_alt() {
        let result = parse_permission_mode("bypass_permissions");
        assert!(matches!(result, PermissionMode::BypassPermissions));
    }

    #[test]
    fn parse_permission_mode_unknown_defaults() {
        let result = parse_permission_mode("unknown");
        assert!(matches!(result, PermissionMode::Default));
    }

    #[test]
    fn parse_permission_mode_case_insensitive() {
        assert!(matches!(
            parse_permission_mode("AUTO"),
            PermissionMode::Auto
        ));
        assert!(matches!(
            parse_permission_mode("DONT_ASK"),
            PermissionMode::DontAsk
        ));
        assert!(matches!(
            parse_permission_mode("BYPASS"),
            PermissionMode::BypassPermissions
        ));
    }
}
