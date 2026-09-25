//! [`Tool`] -- the tools an agent may be allowed or denied.

use strum::Display;

/// A tool an agent can call, for [`AgentConfig::allow_tool`](super::AgentConfig::allow_tool)
/// and [`AgentConfig::disallowed_tools`](super::AgentConfig::disallowed_tools).
///
/// The known variants are the Claude Code built-in tools; a misspelled
/// variant does not compile, where a misspelled string is silently ignored by
/// the CLI. [`Custom`](Tool::Custom) carries anything else verbatim: an MCP
/// tool (`mcp__server__tool`) or a permission pattern (`Bash(git log:*)`).
///
/// # Examples
///
/// ```
/// use ironflow_core::provider::Tool;
///
/// assert_eq!(Tool::WebSearch.to_string(), "WebSearch");
/// assert_eq!(Tool::Custom("mcp__github__search".to_string()).to_string(), "mcp__github__search");
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Hash, Display)]
pub enum Tool {
    /// Run shell commands.
    Bash,
    /// Read files.
    Read,
    /// Create or overwrite files.
    Write,
    /// Edit files in place.
    Edit,
    /// Find files by glob pattern.
    Glob,
    /// Search file contents.
    Grep,
    /// Fetch a URL.
    WebFetch,
    /// Search the web.
    WebSearch,
    /// Edit Jupyter notebooks.
    NotebookEdit,
    /// Maintain a todo list.
    TodoWrite,
    /// Delegate to a sub-agent.
    Task,
    /// Any other tool name or permission pattern, passed as is.
    #[strum(transparent)]
    Custom(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_tools_use_the_claude_code_names() {
        let names: Vec<String> = [
            Tool::Bash,
            Tool::Read,
            Tool::Write,
            Tool::Edit,
            Tool::Glob,
            Tool::Grep,
            Tool::WebFetch,
            Tool::WebSearch,
            Tool::NotebookEdit,
            Tool::TodoWrite,
            Tool::Task,
        ]
        .iter()
        .map(Tool::to_string)
        .collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "Bash",
                "Read",
                "Write",
                "Edit",
                "Glob",
                "Grep",
                "WebFetch",
                "WebSearch",
                "NotebookEdit",
                "TodoWrite",
                "Task"
            ]
        );
    }

    #[test]
    fn custom_tools_are_passed_verbatim() {
        assert_eq!(
            Tool::Custom("mcp__github__search".to_string()).to_string(),
            "mcp__github__search"
        );
        assert_eq!(
            Tool::Custom("Bash(git log:*)".to_string()).to_string(),
            "Bash(git log:*)"
        );
        assert_eq!(Tool::Custom("é".to_string()).to_string(), "é");
    }
}
