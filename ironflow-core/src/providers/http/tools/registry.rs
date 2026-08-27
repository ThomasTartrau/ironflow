//! Tool registry: stores tools and converts them to OpenAI format.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::tool_trait::{Tool, ToolError, ToolOutput};

/// Registry of client-side tools available to the HTTP agent provider.
///
/// Converts registered tools to the OpenAI `tools` array format and routes
/// tool call execution to the correct tool implementation.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::http::tools::ToolRegistry;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let registry = ToolRegistry::new();
/// // registry.register(my_tool);
/// let tools_json = registry.to_openai_tools();
/// # Ok(())
/// # }
/// ```
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
    index: HashMap<String, usize>,
    connectors: HashSet<String>,
}

impl ToolRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            tools: Vec::new(),
            index: HashMap::new(),
            connectors: HashSet::new(),
        }
    }

    /// Register a tool. Panics if a tool with the same name is already registered.
    ///
    /// # Panics
    ///
    /// Panics if a tool with the same name already exists in the registry.
    pub fn register(mut self, tool: impl Tool + 'static) -> Self {
        let name = tool.name().to_string();
        assert!(
            !self.index.contains_key(&name),
            "tool '{}' already registered",
            name
        );
        let idx = self.tools.len();
        self.tools.push(Box::new(tool));
        self.index.insert(name, idx);
        self
    }

    /// Returns the number of registered tools.
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Returns `true` if no tools are registered.
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }

    /// Convert all registered tools to the OpenAI `tools` array format.
    ///
    /// Each tool is represented as:
    /// ```json
    /// {
    ///   "type": "function",
    ///   "function": {
    ///     "name": "tool_name",
    ///     "description": "tool description",
    ///     "parameters": { ... json schema ... }
    ///   }
    /// }
    /// ```
    pub fn to_openai_tools(&self) -> Vec<Value> {
        self.tools
            .iter()
            .map(|tool| {
                json!({
                    "type": "function",
                    "function": {
                        "name": tool.name(),
                        "description": tool.description(),
                        "parameters": tool.parameters_schema()
                    }
                })
            })
            .collect()
    }

    /// Execute a tool by name with the given input.
    ///
    /// Returns `None` if no tool with that name is registered.
    pub async fn execute(&self, name: &str, input: Value) -> Option<Result<ToolOutput, ToolError>> {
        let idx = self.index.get(name)?;
        let tool = &self.tools[*idx];
        Some(tool.execute(input).await)
    }

    /// Check if a tool with the given name is registered.
    pub fn has_tool(&self, name: &str) -> bool {
        self.index.contains_key(name)
    }

    /// Register a connector name for MCP prefix routing.
    ///
    /// Called by `register_mcp_tools` to track which prefixes are valid for
    /// tool call routing.
    pub fn register_connector(mut self, name: &str) -> Self {
        self.connectors.insert(name.to_string());
        self
    }

    /// Returns the set of registered MCP connector prefixes.
    pub fn connectors(&self) -> &HashSet<String> {
        &self.connectors
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use serde_json::json;

    use super::*;

    struct AddTool;

    impl Tool for AddTool {
        fn name(&self) -> &str {
            "add"
        }
        fn description(&self) -> &str {
            "Adds two numbers"
        }
        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "properties": {
                    "a": {"type": "number"},
                    "b": {"type": "number"}
                },
                "required": ["a", "b"]
            })
        }
        fn execute(
            &self,
            input: Value,
        ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
            Box::pin(async move {
                let a = input
                    .get("a")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| ToolError::new("missing 'a'"))?;
                let b = input
                    .get("b")
                    .and_then(|v| v.as_f64())
                    .ok_or_else(|| ToolError::new("missing 'b'"))?;
                Ok(ToolOutput::success(format!("{}", a + b)))
            })
        }
    }

    struct EchoTool;

    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input"
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object", "properties": {"msg": {"type": "string"}}})
        }
        fn execute(
            &self,
            input: Value,
        ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
            Box::pin(async move {
                let msg = input.get("msg").and_then(|v| v.as_str()).unwrap_or("empty");
                Ok(ToolOutput::success(msg))
            })
        }
    }

    #[test]
    fn registry_new_is_empty() {
        let registry = ToolRegistry::new();
        assert!(registry.is_empty());
        assert_eq!(registry.len(), 0);
    }

    #[test]
    fn registry_register_increments_count() {
        let registry = ToolRegistry::new().register(AddTool).register(EchoTool);
        assert_eq!(registry.len(), 2);
        assert!(!registry.is_empty());
    }

    #[test]
    fn registry_has_tool() {
        let registry = ToolRegistry::new().register(AddTool);
        assert!(registry.has_tool("add"));
        assert!(!registry.has_tool("echo"));
    }

    #[test]
    #[should_panic(expected = "tool 'add' already registered")]
    fn registry_duplicate_panics() {
        ToolRegistry::new().register(AddTool).register(AddTool);
    }

    #[test]
    fn to_openai_tools_format() {
        let registry = ToolRegistry::new().register(AddTool);
        let tools = registry.to_openai_tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["type"], "function");
        assert_eq!(tools[0]["function"]["name"], "add");
        assert_eq!(tools[0]["function"]["description"], "Adds two numbers");
        assert_eq!(tools[0]["function"]["parameters"]["type"], "object");
        assert!(tools[0]["function"]["parameters"]["properties"]["a"].is_object());
    }

    #[test]
    fn to_openai_tools_multiple() {
        let registry = ToolRegistry::new().register(AddTool).register(EchoTool);
        let tools = registry.to_openai_tools();

        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["function"]["name"], "add");
        assert_eq!(tools[1]["function"]["name"], "echo");
    }

    #[tokio::test]
    async fn execute_existing_tool() {
        let registry = ToolRegistry::new().register(AddTool);
        let result = registry
            .execute("add", json!({"a": 3, "b": 4}))
            .await
            .expect("tool should exist")
            .expect("tool should succeed");

        assert_eq!(result.content, "7");
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn execute_nonexistent_tool_returns_none() {
        let registry = ToolRegistry::new().register(AddTool);
        let result = registry.execute("nonexistent", json!({})).await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn execute_tool_error_propagates() {
        let registry = ToolRegistry::new().register(AddTool);
        let result = registry
            .execute("add", json!({"a": 1}))
            .await
            .expect("tool should exist");

        assert!(result.is_err());
    }
}
