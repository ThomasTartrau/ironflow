//! [`AgentStepConfig`] — serializable configuration for an agent step.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Serializable configuration for an agent step.
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStepConfig {
    /// The user prompt.
    pub prompt: String,
    /// Optional system prompt.
    pub system_prompt: Option<String>,
    /// Model name (e.g. "sonnet", "opus", "haiku").
    pub model: Option<String>,
    /// Maximum budget in USD.
    pub max_budget_usd: Option<f64>,
    /// Maximum number of agentic turns.
    pub max_turns: Option<u32>,
    /// Tool allowlist.
    pub allowed_tools: Vec<String>,
    /// Working directory for the agent.
    pub working_dir: Option<String>,
    /// Permission mode (e.g. "auto", "dont_ask").
    pub permission_mode: Option<String>,
    /// Optional JSON Schema string for structured output.
    ///
    /// When set, the agent provider will request typed output conforming to this schema.
    /// The result value is guaranteed to be valid JSON matching the schema.
    pub output_schema: Option<String>,
}

impl AgentStepConfig {
    /// Create a new agent config with the given prompt.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::AgentStepConfig;
    ///
    /// let config = AgentStepConfig::new("Summarize this file");
    /// assert_eq!(config.prompt, "Summarize this file");
    /// ```
    pub fn new(prompt: &str) -> Self {
        Self {
            prompt: prompt.to_string(),
            system_prompt: None,
            model: None,
            max_budget_usd: None,
            max_turns: None,
            allowed_tools: Vec::new(),
            working_dir: None,
            permission_mode: None,
            output_schema: None,
        }
    }

    /// Set the system prompt.
    pub fn system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = Some(prompt.to_string());
        self
    }

    /// Set the model name.
    pub fn model(mut self, model: &str) -> Self {
        self.model = Some(model.to_string());
        self
    }

    /// Set the maximum budget in USD.
    pub fn max_budget_usd(mut self, budget: f64) -> Self {
        self.max_budget_usd = Some(budget);
        self
    }

    /// Set the maximum number of turns.
    pub fn max_turns(mut self, turns: u32) -> Self {
        self.max_turns = Some(turns);
        self
    }

    /// Add an allowed tool.
    pub fn allow_tool(mut self, tool: &str) -> Self {
        self.allowed_tools.push(tool.to_string());
        self
    }

    /// Set the working directory.
    pub fn working_dir(mut self, dir: &str) -> Self {
        self.working_dir = Some(dir.to_string());
        self
    }

    /// Set the permission mode.
    pub fn permission_mode(mut self, mode: &str) -> Self {
        self.permission_mode = Some(mode.to_string());
        self
    }

    /// Set structured output from a Rust type implementing [`JsonSchema`].
    ///
    /// The schema is serialized once at build time. When set, the agent provider
    /// will request typed output conforming to this schema.
    ///
    /// **Important:** structured output requires `max_turns >= 2`. The Claude CLI
    /// uses the first turn for reasoning and a second turn to produce the
    /// schema-conforming JSON. If `max_turns` is set to `1`, the agent will
    /// fail at runtime with an `error_max_turns` error.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::AgentStepConfig;
    /// use schemars::JsonSchema;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize, JsonSchema)]
    /// struct Labels {
    ///     labels: Vec<String>,
    /// }
    ///
    /// let config = AgentStepConfig::new("Classify this email")
    ///     .output::<Labels>()
    ///     .max_turns(2);
    ///
    /// assert!(config.output_schema.is_some());
    /// ```
    pub fn output<T: JsonSchema>(mut self) -> Self {
        let schema = schemars::schema_for!(T);
        self.output_schema = match serde_json::to_string(&schema) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    type_name = std::any::type_name::<T>(),
                    "failed to serialize JSON schema, structured output disabled"
                );
                None
            }
        };
        self
    }

    /// Set structured output from a pre-serialized JSON Schema string.
    ///
    /// Use this when the schema comes from configuration (e.g. YAML/JSON files)
    /// rather than a Rust type. For type-safe schema generation, prefer
    /// [`output`](AgentStepConfig::output).
    ///
    /// **Important:** structured output requires `max_turns >= 2`. See
    /// [`output`](AgentStepConfig::output) for details.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::AgentStepConfig;
    ///
    /// let schema = r#"{"type":"object","properties":{"score":{"type":"integer"}}}"#;
    /// let config = AgentStepConfig::new("Rate this PR")
    ///     .output_schema_raw(schema.to_string());
    ///
    /// assert_eq!(config.output_schema.as_deref(), Some(schema));
    /// ```
    pub fn output_schema_raw(mut self, schema: String) -> Self {
        self.output_schema = Some(schema);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder() {
        let config = AgentStepConfig::new("Review code")
            .system_prompt("You are a code reviewer")
            .model("haiku")
            .max_budget_usd(0.50)
            .max_turns(5)
            .allow_tool("read")
            .working_dir("/repo")
            .permission_mode("auto");

        assert_eq!(config.prompt, "Review code");
        assert_eq!(config.system_prompt.unwrap(), "You are a code reviewer");
        assert_eq!(config.model.unwrap(), "haiku");
        assert_eq!(config.allowed_tools, vec!["read"]);
        assert!(config.output_schema.is_none());
    }

    #[test]
    fn output_sets_schema_from_type() {
        #[derive(serde::Deserialize, JsonSchema)]
        #[allow(dead_code)]
        struct Labels {
            labels: Vec<String>,
        }

        let config = AgentStepConfig::new("Classify").output::<Labels>();

        let schema = config.output_schema.expect("schema should be set");
        assert!(schema.contains("labels"));
    }

    #[test]
    fn output_schema_raw_sets_string() {
        let raw = r#"{"type":"object"}"#;
        let config = AgentStepConfig::new("Rate").output_schema_raw(raw.to_string());

        assert_eq!(config.output_schema.as_deref(), Some(raw));
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

        let schema = config.output_schema.expect("schema should be set");
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
            .output_schema_raw(raw.to_string());

        assert_eq!(config.output_schema.as_deref(), Some(raw));
    }

    #[test]
    fn default_output_schema_is_none() {
        let config = AgentStepConfig::new("Hello");
        assert!(config.output_schema.is_none());
    }
}
