//! [`AgentStepConfig`] — serializable configuration for an agent step.

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
    }
}
