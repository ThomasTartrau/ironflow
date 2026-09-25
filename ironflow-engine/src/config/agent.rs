//! Agent step configuration -- re-exports [`AgentConfig`] from `ironflow-core`.
//!
//! [`AgentStepConfig`] is a type alias for [`AgentConfig`], keeping backward
//! compatibility while eliminating the duplicated config struct.
//! [`AgentStep`] ties each typestate of the builder to what
//! [`WorkflowContext::agent`](crate::context::WorkflowContext::agent) returns.

use serde::de::DeserializeOwned;

pub use ironflow_core::provider::{AgentConfig, Tool};
use ironflow_core::provider::{NoSchema, NoTools, RawSchema, WithSchema, WithTools};

use crate::error::EngineError;
use crate::executor::StepOutput;

/// Backward-compatible alias for [`AgentConfig`].
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::AgentStepConfig;
///
/// let config = AgentStepConfig::new("Review this code for security issues")
///     .model("haiku")
///     .max_budget_usd(0.10);
/// ```
pub type AgentStepConfig = AgentConfig;

/// An agent configuration [`WorkflowContext::agent`](crate::context::WorkflowContext::agent)
/// accepts, and the answer it returns for it.
///
/// A config built with [`output::<T>()`](AgentConfig::output) answers with the
/// `T` itself; every other config answers with the raw [`StepOutput`].
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::{AgentStep, AgentStepConfig};
/// use ironflow_engine::executor::{StepArtifacts, StepOutput};
/// use rust_decimal::Decimal;
/// use schemars::JsonSchema;
/// use serde::Deserialize;
/// use serde_json::json;
///
/// #[derive(Deserialize, JsonSchema)]
/// struct Verdict {
///     approved: bool,
/// }
///
/// let config = AgentStepConfig::new("Review").max_turns(2).output::<Verdict>();
/// let output = StepOutput {
///     output: json!({"approved": true}),
///     duration_ms: 0,
///     cost_usd: Decimal::ZERO,
///     input_tokens: None,
///     cache_read_input_tokens: None,
///     cache_creation_input_tokens: None,
///     output_tokens: None,
///     model: None,
///     debug_messages: None,
///     artifacts: StepArtifacts::default(),
/// };
/// # fn answer<C: AgentStep>(_config: &C, output: StepOutput) -> Result<C::Answer, ironflow_engine::error::EngineError> {
/// #     C::answer(output)
/// # }
/// let verdict: Verdict = answer(&config, output)?;
/// assert!(verdict.approved);
/// # Ok::<(), ironflow_engine::error::EngineError>(())
/// ```
pub trait AgentStep {
    /// What the step returns to the handler.
    type Answer;

    /// The configuration, without its typestate.
    fn into_config(self) -> AgentStepConfig;

    /// Read the step output as the answer.
    ///
    /// # Errors
    ///
    /// Returns [`EngineError::Serialization`] when a typed answer does not
    /// match its type.
    fn answer(output: StepOutput) -> Result<Self::Answer, EngineError>;
}

impl AgentStep for AgentConfig<NoTools, NoSchema> {
    type Answer = StepOutput;

    fn into_config(self) -> AgentStepConfig {
        self
    }

    fn answer(output: StepOutput) -> Result<StepOutput, EngineError> {
        Ok(output)
    }
}

impl AgentStep for AgentConfig<WithTools, NoSchema> {
    type Answer = StepOutput;

    fn into_config(self) -> AgentStepConfig {
        self.into()
    }

    fn answer(output: StepOutput) -> Result<StepOutput, EngineError> {
        Ok(output)
    }
}

impl AgentStep for AgentConfig<NoTools, RawSchema> {
    type Answer = StepOutput;

    fn into_config(self) -> AgentStepConfig {
        self.into()
    }

    fn answer(output: StepOutput) -> Result<StepOutput, EngineError> {
        Ok(output)
    }
}

impl<T: DeserializeOwned> AgentStep for AgentConfig<NoTools, WithSchema<T>> {
    type Answer = T;

    fn into_config(self) -> AgentStepConfig {
        self.into()
    }

    fn answer(output: StepOutput) -> Result<T, EngineError> {
        output.json()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::StepArtifacts;
    use rust_decimal::Decimal;
    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::{Value, json};

    fn step_output(output: Value) -> StepOutput {
        StepOutput {
            output,
            duration_ms: 0,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            cache_read_input_tokens: None,
            cache_creation_input_tokens: None,
            output_tokens: None,
            model: None,
            debug_messages: None,
            artifacts: StepArtifacts::default(),
        }
    }

    #[derive(Debug, PartialEq, Deserialize, JsonSchema)]
    struct Verdict {
        approved: bool,
    }

    #[test]
    fn a_typed_config_answers_with_its_type() {
        let answer = <AgentConfig<NoTools, WithSchema<Verdict>> as AgentStep>::answer(step_output(
            json!({"approved": true}),
        ))
        .expect("matches Verdict");
        assert_eq!(answer, Verdict { approved: true });
    }

    #[test]
    fn a_typed_config_rejects_an_answer_of_another_shape() {
        let err =
            <AgentConfig<NoTools, WithSchema<Verdict>> as AgentStep>::answer(step_output(json!([
                "approved"
            ])))
            .expect_err("not a Verdict");
        assert!(matches!(err, EngineError::Serialization(_)));
    }

    #[test]
    fn untyped_configs_answer_with_the_raw_output() {
        let raw = <AgentConfig as AgentStep>::answer(step_output(json!("free text")))
            .expect("raw output");
        assert_eq!(raw.output, json!("free text"));
    }

    #[test]
    fn into_config_keeps_the_settings() {
        let config = AgentStepConfig::new("Review")
            .max_turns(2)
            .output::<Verdict>()
            .into_config();
        assert_eq!(config.max_turns, Some(2));
        assert!(config.json_schema.is_some());

        let config = AgentStepConfig::new("Explore")
            .allow_tool(Tool::Grep)
            .into_config();
        assert_eq!(config.allowed_tools, vec!["Grep"]);
    }

    #[test]
    fn builder() {
        let config = AgentStepConfig::new("Review code")
            .system_prompt("You are a code reviewer")
            .model("haiku")
            .max_budget_usd(0.50)
            .max_turns(5)
            .allow_tool(Tool::Read)
            .working_dir("/repo")
            .permission_mode(ironflow_core::operations::agent::PermissionMode::Auto);

        assert_eq!(config.prompt, "Review code");
        assert_eq!(config.system_prompt.unwrap(), "You are a code reviewer");
        assert_eq!(config.model, "haiku");
        assert_eq!(config.allowed_tools, vec!["Read"]);
        assert!(config.json_schema.is_none());
    }

    #[test]
    fn output_sets_schema_from_type() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Labels {
            labels: Vec<String>,
        }

        let config = AgentStepConfig::new("Classify").output::<Labels>();

        let schema = config.json_schema.expect("schema should be set");
        assert!(schema.contains("labels"));
    }

    #[test]
    fn output_schema_raw_sets_string() {
        let raw = r#"{"type":"object"}"#;
        let config = AgentStepConfig::new("Rate").output_schema_raw(raw);

        assert_eq!(config.json_schema.as_deref(), Some(raw));
    }

    #[test]
    fn output_overrides_previous_schema() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct First {
            a: String,
        }

        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Second {
            b: i32,
        }

        let config = AgentStepConfig::new("Test")
            .output::<First>()
            .output::<Second>();

        let schema = config.json_schema.expect("schema should be set");
        assert!(!schema.contains("\"a\""));
        assert!(schema.contains("\"b\""));
    }

    #[test]
    fn output_schema_raw_overrides_typed_schema() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Typed {
            field: String,
        }

        let raw = r#"{"type":"string"}"#;
        let config = AgentStepConfig::new("Test")
            .output::<Typed>()
            .output_schema_raw(raw);

        assert_eq!(config.json_schema.as_deref(), Some(raw));
    }

    #[test]
    fn default_output_schema_is_none() {
        let config = AgentStepConfig::new("Hello");
        assert!(config.json_schema.is_none());
    }

    #[test]
    fn serde_roundtrip_with_defaults() {
        let json = r#"{"prompt":"hello"}"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.prompt, "hello");
        assert_eq!(config.model, "sonnet");
        assert!(!config.verbose);
    }

    #[test]
    fn serde_permission_mode_case_insensitive() {
        let json = r#"{"prompt":"test","permission_mode":"auto"}"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert!(matches!(
            config.permission_mode,
            ironflow_core::operations::agent::PermissionMode::Auto
        ));
    }

    #[test]
    fn serde_output_schema_alias() {
        let json = r#"{"prompt":"test","output_schema":"{\"type\":\"object\"}"}"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.json_schema.as_deref(), Some(r#"{"type":"object"}"#));
    }

    #[test]
    fn strict_mcp_config_defaults_to_false() {
        let config = AgentStepConfig::new("test");
        assert!(!config.strict_mcp_config);
    }

    #[test]
    fn strict_mcp_config_builder_sets_flag() {
        let config = AgentStepConfig::new("test").strict_mcp_config(true);
        assert!(config.strict_mcp_config);
    }

    #[test]
    fn strict_mcp_config_serde_default_when_missing() {
        let json = r#"{"prompt":"test"}"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert!(!config.strict_mcp_config);
    }

    #[test]
    fn strict_mcp_config_serde_roundtrip() {
        let json = r#"{"prompt":"test","strict_mcp_config":true}"#;
        let config: AgentConfig = serde_json::from_str(json).unwrap();
        assert!(config.strict_mcp_config);
    }
}
