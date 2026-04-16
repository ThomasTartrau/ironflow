//! Agent step configuration -- re-exports [`AgentConfig`] from `ironflow-core`.
//!
//! [`AgentStepConfig`] is a type alias for [`AgentConfig`], keeping backward
//! compatibility while eliminating the duplicated config struct.

pub use ironflow_core::provider::AgentConfig;

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

#[cfg(test)]
mod tests {
    use super::*;
    use schemars::JsonSchema;

    #[test]
    fn builder() {
        let config = AgentStepConfig::new("Review code")
            .system_prompt("You are a code reviewer")
            .model("haiku")
            .max_budget_usd(0.50)
            .max_turns(5)
            .allow_tool("read")
            .working_dir("/repo")
            .permission_mode(ironflow_core::operations::agent::PermissionMode::Auto);

        assert_eq!(config.prompt, "Review code");
        assert_eq!(config.system_prompt.unwrap(), "You are a code reviewer");
        assert_eq!(config.model, "haiku");
        assert_eq!(config.allowed_tools, vec!["read"]);
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
