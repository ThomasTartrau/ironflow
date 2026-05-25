//! The [`Tool`] trait and associated types for client-side tool execution.

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use serde_json::Value;

/// Output of a successful tool execution.
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Content returned to the model (typically text or JSON).
    pub content: String,
    /// Whether this result represents an error that should be reported to the model.
    pub is_error: bool,
}

impl ToolOutput {
    /// Create a successful tool output.
    pub fn success(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: false,
        }
    }

    /// Create an error output that will be reported to the model.
    pub fn error(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            is_error: true,
        }
    }
}

/// Error returned when a tool cannot execute at all (infrastructure failure).
///
/// Distinguished from [`ToolOutput::is_error`] which reports tool-level errors
/// back to the model. A `ToolError` aborts the agentic loop entirely.
#[derive(Debug)]
pub struct ToolError {
    /// Human-readable error message.
    pub message: String,
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "tool execution failed: {}", self.message)
    }
}

impl std::error::Error for ToolError {}

impl ToolError {
    /// Create a new tool error.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

/// A client-side tool that can be executed by the HTTP agent provider.
///
/// Implement this trait to add custom tools to the agentic loop. The tool's
/// JSON schema is sent to the model in the OpenAI `tools` format, and when
/// the model calls the tool, [`execute`](Tool::execute) is invoked with the
/// parsed arguments.
///
/// # Examples
///
/// ```no_run
/// use std::pin::Pin;
/// use std::future::Future;
/// use serde_json::{Value, json};
/// use ironflow_core::providers::http::tools::{Tool, ToolOutput, ToolError};
///
/// struct EchoTool;
///
/// impl Tool for EchoTool {
///     fn name(&self) -> &str { "echo" }
///     fn description(&self) -> &str { "Echoes the input back" }
///     fn parameters_schema(&self) -> Value {
///         json!({
///             "type": "object",
///             "properties": {
///                 "message": { "type": "string", "description": "The message to echo" }
///             },
///             "required": ["message"]
///         })
///     }
///     fn execute(&self, input: Value) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
///         Box::pin(async move {
///             let msg = input.get("message")
///                 .and_then(|v| v.as_str())
///                 .unwrap_or("");
///             Ok(ToolOutput::success(msg))
///         })
///     }
/// }
/// ```
pub trait Tool: Send + Sync {
    /// Unique name of this tool (used in tool_calls routing).
    fn name(&self) -> &str;

    /// Short description shown to the model.
    fn description(&self) -> &str;

    /// JSON Schema for the tool's input parameters.
    fn parameters_schema(&self) -> Value;

    /// Execute the tool with the given input arguments.
    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>>;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    struct FakeTool;

    impl Tool for FakeTool {
        fn name(&self) -> &str {
            "fake"
        }
        fn description(&self) -> &str {
            "A fake tool for testing"
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }
        fn execute(
            &self,
            _input: Value,
        ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
            Box::pin(async { Ok(ToolOutput::success("done")) })
        }
    }

    #[test]
    fn tool_output_success() {
        let output = ToolOutput::success("hello");
        assert_eq!(output.content, "hello");
        assert!(!output.is_error);
    }

    #[test]
    fn tool_output_error() {
        let output = ToolOutput::error("file not found");
        assert_eq!(output.content, "file not found");
        assert!(output.is_error);
    }

    #[test]
    fn tool_error_display() {
        let err = ToolError::new("timeout");
        assert_eq!(err.to_string(), "tool execution failed: timeout");
    }

    #[test]
    fn fake_tool_implements_trait() {
        let tool = FakeTool;
        assert_eq!(tool.name(), "fake");
        assert_eq!(tool.description(), "A fake tool for testing");
        assert_eq!(
            tool.parameters_schema(),
            json!({"type": "object", "properties": {}})
        );
    }

    #[tokio::test]
    async fn fake_tool_execute() {
        let tool = FakeTool;
        let result = tool
            .execute(json!({}))
            .await
            .expect("tool execution should succeed");
        assert_eq!(result.content, "done");
        assert!(!result.is_error);
    }
}
