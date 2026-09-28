//! Tool filtering applied when registering MCP tools into a [`ToolRegistry`](super::ToolRegistry).

use std::collections::HashSet;

use super::protocol::McpToolDef;

/// Filter controlling which tools [`register_mcp_tools_filtered`](super::register_mcp_tools_filtered) registers.
///
/// Denies everything by default: at least one of [`McpToolFilter::allow`] or
/// [`McpToolFilter::require_read_only_hint`] must be called for any tool to
/// be registered. When both are configured, a tool must satisfy both to pass.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
///
/// let filter = McpToolFilter::new()
///     .allow(&["list_incidents", "get_incident"])
///     .require_read_only_hint();
/// ```
#[derive(Debug, Clone, Default)]
pub struct McpToolFilter {
    allow: Option<HashSet<String>>,
    require_read_only_hint: bool,
}

impl McpToolFilter {
    /// Create a filter that denies every tool until configured.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
    ///
    /// let filter = McpToolFilter::new();
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Restrict registration to the given MCP tool names.
    ///
    /// Can be called multiple times (names accumulate) and combined with
    /// [`McpToolFilter::require_read_only_hint`]. A name absent from the
    /// server at registration time makes
    /// [`register_mcp_tools_filtered`](super::register_mcp_tools_filtered)
    /// fail with [`McpError::ToolNotFound`](super::McpError::ToolNotFound).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
    ///
    /// let filter = McpToolFilter::new().allow(&["list_incidents", "get_incident"]);
    /// ```
    pub fn allow(mut self, names: &[&str]) -> Self {
        self.allow
            .get_or_insert_with(HashSet::new)
            .extend(names.iter().map(|name| (*name).to_string()));
        self
    }

    /// Reject any tool without a `readOnlyHint: true` annotation.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
    ///
    /// let filter = McpToolFilter::new().require_read_only_hint();
    /// ```
    pub fn require_read_only_hint(mut self) -> Self {
        self.require_read_only_hint = true;
        self
    }

    /// Names explicitly allow-listed, if [`McpToolFilter::allow`] was called at least once.
    pub(super) fn allowed_names(&self) -> Option<&HashSet<String>> {
        self.allow.as_ref()
    }

    /// Whether `tool` satisfies every restriction configured on this filter.
    ///
    /// Returns `false` when no restriction is configured at all, enforcing
    /// default-deny.
    pub(super) fn matches(&self, tool: &McpToolDef) -> bool {
        let mut has_restriction = false;

        if let Some(allow) = &self.allow {
            has_restriction = true;
            if !allow.contains(&tool.name) {
                return false;
            }
        }

        if self.require_read_only_hint {
            has_restriction = true;
            let read_only = tool.annotations.as_ref().is_some_and(|a| a.read_only_hint);
            if !read_only {
                return false;
            }
        }

        has_restriction
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::McpToolAnnotations;
    use super::*;

    fn tool(name: &str, read_only_hint: Option<bool>) -> McpToolDef {
        McpToolDef {
            name: name.to_string(),
            description: None,
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
            annotations: read_only_hint.map(|read_only_hint| McpToolAnnotations { read_only_hint }),
        }
    }

    #[test]
    fn default_filter_matches_nothing() {
        let filter = McpToolFilter::new();
        assert!(!filter.matches(&tool("anything", None)));
    }

    #[test]
    fn allow_keeps_only_listed_names() {
        let filter = McpToolFilter::new().allow(&["a", "b"]);
        assert!(filter.matches(&tool("a", None)));
        assert!(filter.matches(&tool("b", None)));
        assert!(!filter.matches(&tool("c", None)));
    }

    #[test]
    fn allow_extends_across_calls() {
        let filter = McpToolFilter::new().allow(&["a"]).allow(&["b"]);
        assert!(filter.matches(&tool("a", None)));
        assert!(filter.matches(&tool("b", None)));
    }

    #[test]
    fn require_read_only_hint_rejects_missing_or_false_hint() {
        let filter = McpToolFilter::new().require_read_only_hint();
        assert!(filter.matches(&tool("safe", Some(true))));
        assert!(!filter.matches(&tool("writer", Some(false))));
        assert!(!filter.matches(&tool("unmarked", None)));
    }

    #[test]
    fn allow_and_require_read_only_hint_combine() {
        let filter = McpToolFilter::new()
            .allow(&["safe", "writer"])
            .require_read_only_hint();
        assert!(filter.matches(&tool("safe", Some(true))));
        assert!(!filter.matches(&tool("writer", Some(false))));
        assert!(!filter.matches(&tool("other", Some(true))));
    }

    #[test]
    fn allowed_names_reflects_configured_allow_list() {
        assert!(McpToolFilter::new().allowed_names().is_none());
        let filter = McpToolFilter::new().allow(&["a"]);
        assert_eq!(filter.allowed_names().unwrap().len(), 1);
    }
}
