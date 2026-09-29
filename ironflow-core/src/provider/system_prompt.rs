//! Appending to the system prompt instead of replacing it.

use super::AgentConfig;

impl<Tools, Schema> AgentConfig<Tools, Schema> {
    /// Append `prompt` to the system prompt instead of replacing it.
    ///
    /// Use it to give Claude Code project rules while keeping its default
    /// system prompt (skills, slash commands, tools), which
    /// [`system_prompt`](Self::system_prompt) would replace. The Claude CLI
    /// receives it as `--append-system-prompt`; HTTP providers send
    /// [`full_system_prompt`](Self::full_system_prompt).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("/code-review medium main...HEAD")
    ///     .append_system_prompt("Never hardcode a tenant id.");
    /// assert_eq!(
    ///     config.append_system_prompt.as_deref(),
    ///     Some("Never hardcode a tenant id."),
    /// );
    /// ```
    pub fn append_system_prompt(mut self, prompt: &str) -> Self {
        self.append_system_prompt = Some(prompt.to_string());
        self
    }

    /// The system prompt a provider without an append flag sends:
    /// [`system_prompt`](Self::system_prompt) then
    /// [`append_system_prompt`](Self::append_system_prompt), separated by a
    /// blank line. `None` when neither is set.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::provider::AgentConfig;
    ///
    /// let config = AgentConfig::new("hi")
    ///     .system_prompt("Be brief")
    ///     .append_system_prompt("Rules");
    /// assert_eq!(config.full_system_prompt().as_deref(), Some("Be brief\n\nRules"));
    /// ```
    pub fn full_system_prompt(&self) -> Option<String> {
        match (&self.system_prompt, &self.append_system_prompt) {
            (Some(base), Some(extra)) => Some(format!("{base}\n\n{extra}")),
            (Some(only), None) | (None, Some(only)) => Some(only.clone()),
            (None, None) => None,
        }
    }
}
