//! MCP tool call routing by connector prefix.
//!
//! Resolves tool names in the `{connector_id}__{tool_name}` format to their
//! connector and tool components. Bare names (without a prefix) are rejected
//! explicitly rather than falling back to any default server.

use std::collections::HashSet;
use std::fmt;

/// Separator between the connector ID and the tool name.
pub const CONNECTOR_SEPARATOR: &str = "__";

/// Errors that can occur during MCP tool routing.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::http::tools::routing::RoutingError;
///
/// let err = RoutingError::ConnectorNotFound {
///     connector: "unknown".to_string(),
///     tool: "search".to_string(),
/// };
/// assert!(err.to_string().contains("unknown"));
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutingError {
    /// The tool name has no connector prefix (bare name).
    AmbiguousToolName {
        /// The bare tool name that was received.
        name: String,
    },
    /// The connector prefix does not match any registered connector.
    ConnectorNotFound {
        /// The connector prefix that was not found.
        connector: String,
        /// The tool name after the prefix.
        tool: String,
    },
}

impl fmt::Display for RoutingError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AmbiguousToolName { name } => {
                write!(f, "tool '{name}' has no connector prefix")
            }
            Self::ConnectorNotFound { connector, tool } => {
                write!(f, "connector '{connector}' not found for tool '{tool}'")
            }
        }
    }
}

impl std::error::Error for RoutingError {}

/// Resolved components of a prefixed tool name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoutedTool {
    /// The connector ID (prefix before `__`).
    pub connector: String,
    /// The tool name (everything after the first `__`).
    pub tool: String,
    /// The full registry key (`connector__tool`).
    pub registry_key: String,
}

/// Parse and validate a tool call name against a set of known connectors.
///
/// Resolution follows two tiers:
/// 1. **Prefixed name** (`connector__tool`): extracts the prefix, checks it
///    exists in `known_connectors`.
/// 2. **Bare name** (`tool`): rejected with [`RoutingError::AmbiguousToolName`].
///
/// Nested underscores are preserved: `prefix__deep__search` resolves to
/// connector `prefix` and tool `deep__search`.
///
/// # Errors
///
/// - [`RoutingError::AmbiguousToolName`] if the name has no `__` separator.
/// - [`RoutingError::ConnectorNotFound`] if the prefix is not in `known_connectors`.
///
/// # Examples
///
/// ```
/// use std::collections::HashSet;
/// use ironflow_core::providers::http::tools::routing::route_tool_call;
///
/// let connectors: HashSet<String> = ["grafana"].iter().map(|s| s.to_string()).collect();
///
/// let routed = route_tool_call("grafana__query", &connectors).unwrap();
/// assert_eq!(routed.connector, "grafana");
/// assert_eq!(routed.tool, "query");
/// ```
pub fn route_tool_call(
    name: &str,
    known_connectors: &HashSet<String>,
) -> Result<RoutedTool, RoutingError> {
    let Some((connector, tool)) = name.split_once(CONNECTOR_SEPARATOR) else {
        return Err(RoutingError::AmbiguousToolName {
            name: name.to_string(),
        });
    };

    if !known_connectors.contains(connector) {
        return Err(RoutingError::ConnectorNotFound {
            connector: connector.to_string(),
            tool: tool.to_string(),
        });
    }

    Ok(RoutedTool {
        connector: connector.to_string(),
        tool: tool.to_string(),
        registry_key: name.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connectors(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn route_prefixed_tool_resolves() {
        let known = connectors(&["grafana", "slack"]);
        let routed = route_tool_call("grafana__query", &known).unwrap();
        assert_eq!(routed.connector, "grafana");
        assert_eq!(routed.tool, "query");
        assert_eq!(routed.registry_key, "grafana__query");
    }

    #[test]
    fn route_bare_name_rejected() {
        let known = connectors(&["grafana"]);
        let err = route_tool_call("query", &known).unwrap_err();
        assert_eq!(
            err,
            RoutingError::AmbiguousToolName {
                name: "query".to_string()
            }
        );
        assert_eq!(err.to_string(), "tool 'query' has no connector prefix");
    }

    #[test]
    fn route_unknown_connector_rejected() {
        let known = connectors(&["grafana"]);
        let err = route_tool_call("unknown__search", &known).unwrap_err();
        assert_eq!(
            err,
            RoutingError::ConnectorNotFound {
                connector: "unknown".to_string(),
                tool: "search".to_string(),
            }
        );
    }

    #[test]
    fn route_nested_underscores_preserved() {
        let known = connectors(&["prefix"]);
        let routed = route_tool_call("prefix__deep__search", &known).unwrap();
        assert_eq!(routed.connector, "prefix");
        assert_eq!(routed.tool, "deep__search");
    }

    #[test]
    fn route_empty_prefix_rejected() {
        let known = connectors(&["grafana"]);
        let err = route_tool_call("__search", &known).unwrap_err();
        assert_eq!(
            err,
            RoutingError::ConnectorNotFound {
                connector: "".to_string(),
                tool: "search".to_string(),
            }
        );
    }

    #[test]
    fn route_empty_tool_name_resolves_if_connector_known() {
        let known = connectors(&["prefix"]);
        let routed = route_tool_call("prefix__", &known).unwrap();
        assert_eq!(routed.connector, "prefix");
        assert_eq!(routed.tool, "");
    }

    #[test]
    fn display_ambiguous_tool_name() {
        let err = RoutingError::AmbiguousToolName {
            name: "search".to_string(),
        };
        assert_eq!(err.to_string(), "tool 'search' has no connector prefix");
    }

    #[test]
    fn display_connector_not_found() {
        let err = RoutingError::ConnectorNotFound {
            connector: "unknown".to_string(),
            tool: "search".to_string(),
        };
        assert_eq!(
            err.to_string(),
            "connector 'unknown' not found for tool 'search'"
        );
    }
}
