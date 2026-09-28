//! Tool filtering applied when registering MCP tools into a [`ToolRegistry`](super::ToolRegistry).

use std::collections::BTreeSet;

use super::protocol::McpToolDef;

/// Filter controlling which tools [`register_mcp_tools_filtered`](super::register_mcp_tools_filtered) registers.
///
/// A filter always restricts something: it is built with either
/// [`McpToolFilter::allow`] (an explicit list of tool names) or
/// [`McpToolFilter::read_only`] (every tool annotated `readOnlyHint: true`).
/// There is no "allow everything" filter; use
/// [`register_mcp_tools`](super::register_mcp_tools) for that.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
///
/// // Exactly these two tools, and both must be annotated read-only.
/// let filter = McpToolFilter::allow(["list_incidents", "get_incident"])
///     .require_read_only_hint();
///
/// // Every read-only tool the server exposes.
/// let filter = McpToolFilter::read_only();
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpToolFilter {
    allow: Option<BTreeSet<String>>,
    require_read_only_hint: bool,
}

impl McpToolFilter {
    /// Register only the MCP tools with these exact names.
    ///
    /// Names are the tool names the MCP server exposes, without the registry
    /// prefix, and are compared exactly: no glob or regex. Every name must
    /// exist on the server, otherwise
    /// [`register_mcp_tools_filtered`](super::register_mcp_tools_filtered)
    /// fails with [`McpError::ToolNotFound`](super::McpError::ToolNotFound),
    /// so a renamed or removed tool is caught at registration time instead of
    /// during an agent run.
    ///
    /// # Panics
    ///
    /// Panics if `names` is empty: such a filter would register nothing.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
    ///
    /// let filter = McpToolFilter::allow(["list_incidents", "get_incident"]);
    ///
    /// // Names loaded from configuration work too.
    /// let names: Vec<String> = vec!["list_incidents".to_string()];
    /// let filter = McpToolFilter::allow(names);
    /// ```
    ///
    /// ```should_panic
    /// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
    ///
    /// let filter = McpToolFilter::allow(Vec::<String>::new());
    /// ```
    pub fn allow<I, S>(names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let names: BTreeSet<String> = names.into_iter().map(Into::into).collect();
        assert!(
            !names.is_empty(),
            "McpToolFilter::allow requires at least one tool name; use register_mcp_tools to register every tool"
        );
        Self {
            allow: Some(names),
            require_read_only_hint: false,
        }
    }

    /// Register every tool annotated `readOnlyHint: true`, and skip the others.
    ///
    /// Tools without the annotation, or with `readOnlyHint: false`, are left
    /// out and logged at `debug` level.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
    ///
    /// let filter = McpToolFilter::read_only();
    /// ```
    pub fn read_only() -> Self {
        Self {
            allow: None,
            require_read_only_hint: true,
        }
    }

    /// Also require every allowed tool to be annotated `readOnlyHint: true`.
    ///
    /// Combined with [`McpToolFilter::allow`], an allowed tool without the
    /// annotation makes
    /// [`register_mcp_tools_filtered`](super::register_mcp_tools_filtered)
    /// fail with [`McpError::ToolNotReadOnly`](super::McpError::ToolNotReadOnly):
    /// a tool asked for by name is never dropped silently.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::tools::mcp::McpToolFilter;
    ///
    /// let filter = McpToolFilter::allow(["get_incident"]).require_read_only_hint();
    /// ```
    pub fn require_read_only_hint(mut self) -> Self {
        self.require_read_only_hint = true;
        self
    }

    /// Names passed to [`McpToolFilter::allow`], if the filter was built with it.
    pub(super) fn allowed_names(&self) -> Option<&BTreeSet<String>> {
        self.allow.as_ref()
    }

    /// Whether the filter requires a `readOnlyHint: true` annotation.
    pub(super) fn requires_read_only_hint(&self) -> bool {
        self.require_read_only_hint
    }

    /// Whether `tool` satisfies every restriction of this filter.
    pub(super) fn matches(&self, tool: &McpToolDef) -> bool {
        let allowed = self
            .allow
            .as_ref()
            .is_none_or(|names| names.contains(&tool.name));
        allowed && (!self.require_read_only_hint || is_read_only(tool))
    }
}

/// Whether `tool` is annotated `readOnlyHint: true`.
pub(super) fn is_read_only(tool: &McpToolDef) -> bool {
    tool.annotations.as_ref().is_some_and(|a| a.read_only_hint)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::protocol::McpToolAnnotations;
    use super::*;

    fn tool(name: &str, read_only_hint: Option<bool>) -> McpToolDef {
        McpToolDef {
            name: name.to_string(),
            description: None,
            input_schema: json!({"type": "object", "properties": {}}),
            annotations: read_only_hint.map(|read_only_hint| McpToolAnnotations { read_only_hint }),
        }
    }

    #[test]
    fn allow_keeps_only_listed_names() {
        let filter = McpToolFilter::allow(["a", "b"]);
        assert!(filter.matches(&tool("a", None)));
        assert!(filter.matches(&tool("b", None)));
        assert!(!filter.matches(&tool("c", None)));
    }

    #[test]
    fn allow_compares_names_exactly() {
        let filter = McpToolFilter::allow(["list_*"]);
        assert!(!filter.matches(&tool("list_incidents", None)));
        assert!(!filter.matches(&tool("LIST_*", None)));
        assert!(filter.matches(&tool("list_*", None)));
    }

    #[test]
    fn allow_accepts_owned_strings_and_deduplicates() {
        let filter = McpToolFilter::allow(vec!["a".to_string(), "a".to_string()]);
        assert_eq!(filter.allowed_names().map(BTreeSet::len), Some(1));
    }

    #[test]
    fn allow_accepts_unicode_names() {
        let filter = McpToolFilter::allow(["lister_événements"]);
        assert!(filter.matches(&tool("lister_événements", None)));
        assert!(!filter.matches(&tool("lister_evenements", None)));
    }

    #[test]
    #[should_panic(expected = "requires at least one tool name")]
    fn allow_panics_on_empty_list() {
        McpToolFilter::allow(Vec::<&str>::new());
    }

    #[test]
    fn read_only_rejects_missing_or_false_hint() {
        let filter = McpToolFilter::read_only();
        assert!(filter.matches(&tool("safe", Some(true))));
        assert!(!filter.matches(&tool("writer", Some(false))));
        assert!(!filter.matches(&tool("unmarked", None)));
        assert!(filter.allowed_names().is_none());
        assert!(filter.requires_read_only_hint());
    }

    #[test]
    fn allow_and_require_read_only_hint_combine() {
        let filter = McpToolFilter::allow(["safe", "writer"]).require_read_only_hint();
        assert!(filter.matches(&tool("safe", Some(true))));
        assert!(!filter.matches(&tool("writer", Some(false))));
        assert!(!filter.matches(&tool("other", Some(true))));
    }

    #[test]
    fn allow_alone_does_not_require_read_only_hint() {
        let filter = McpToolFilter::allow(["writer"]);
        assert!(!filter.requires_read_only_hint());
        assert!(filter.matches(&tool("writer", Some(false))));
    }

    #[test]
    fn require_read_only_hint_on_read_only_is_idempotent() {
        assert_eq!(
            McpToolFilter::read_only().require_read_only_hint(),
            McpToolFilter::read_only()
        );
    }
}
