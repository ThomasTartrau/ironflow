//! JSON-RPC types for the Model Context Protocol (MCP).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// JSON-RPC 2.0 request.
#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 notification (no id, no response expected).
#[derive(Debug, Serialize)]
pub(crate) struct JsonRpcNotification {
    pub jsonrpc: &'static str,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcNotification {
    pub fn new(method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response.
#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcResponse {
    #[allow(dead_code)]
    pub id: u64,
    pub result: Option<Value>,
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error object.
#[derive(Debug, Deserialize)]
pub(crate) struct JsonRpcError {
    pub code: i64,
    pub message: String,
}

/// An MCP tool definition returned by `tools/list`.
#[derive(Debug, Deserialize)]
pub struct McpToolDef {
    /// Tool name (unique within the server).
    pub name: String,
    /// Human-readable description of the tool.
    pub description: Option<String>,
    /// JSON Schema for the tool's input parameters.
    #[serde(rename = "inputSchema")]
    pub input_schema: Value,
}

/// Result of a `tools/list` call.
#[derive(Debug, Deserialize)]
pub(crate) struct ToolsListResult {
    pub tools: Vec<McpToolDef>,
}

/// Result of a `tools/call` call.
#[derive(Debug, Deserialize)]
pub(crate) struct ToolCallResult {
    pub content: Vec<ContentBlock>,
    #[serde(default)]
    pub is_error: bool,
}

/// A content block in an MCP tool response.
#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
pub(crate) enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "image")]
    Image {
        data: String,
        #[serde(rename = "mimeType")]
        mime_type: String,
    },
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn serialize_request() {
        let req = JsonRpcRequest::new(1, "tools/list", None);
        let json = serde_json::to_value(&req).expect("serialization should succeed");
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["method"], "tools/list");
        assert!(json.get("params").is_none());
    }

    #[test]
    fn serialize_request_with_params() {
        let params = json!({"name": "test", "arguments": {}});
        let req = JsonRpcRequest::new(42, "tools/call", Some(params.clone()));
        let json = serde_json::to_value(&req).expect("serialization should succeed");
        assert_eq!(json["params"], params);
    }

    #[test]
    fn serialize_notification() {
        let notif = JsonRpcNotification::new("notifications/initialized", None);
        let json = serde_json::to_value(&notif).expect("serialization should succeed");
        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["method"], "notifications/initialized");
        assert!(json.get("params").is_none());
    }

    #[test]
    fn deserialize_response_success() {
        let raw = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {"tools": []}
        });
        let resp: JsonRpcResponse =
            serde_json::from_value(raw).expect("deserialization should succeed");
        assert_eq!(resp.id, 1);
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }

    #[test]
    fn deserialize_response_error() {
        let raw = json!({
            "jsonrpc": "2.0",
            "id": 2,
            "error": {"code": -32601, "message": "Method not found"}
        });
        let resp: JsonRpcResponse =
            serde_json::from_value(raw).expect("deserialization should succeed");
        assert_eq!(resp.id, 2);
        assert!(resp.result.is_none());
        let err = resp.error.expect("error should be present");
        assert_eq!(err.code, -32601);
        assert_eq!(err.message, "Method not found");
    }

    #[test]
    fn deserialize_tool_def() {
        let raw = json!({
            "name": "query_dashboards",
            "description": "Query Grafana dashboards",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {"type": "string"}
                },
                "required": ["query"]
            }
        });
        let def: McpToolDef = serde_json::from_value(raw).expect("deserialization should succeed");
        assert_eq!(def.name, "query_dashboards");
        assert_eq!(def.description.as_deref(), Some("Query Grafana dashboards"));
        assert_eq!(def.input_schema["type"], "object");
    }

    #[test]
    fn deserialize_tool_def_no_description() {
        let raw = json!({
            "name": "minimal",
            "inputSchema": {"type": "object", "properties": {}}
        });
        let def: McpToolDef = serde_json::from_value(raw).expect("deserialization should succeed");
        assert_eq!(def.name, "minimal");
        assert!(def.description.is_none());
    }

    #[test]
    fn deserialize_tools_list_result() {
        let raw = json!({
            "tools": [
                {
                    "name": "tool_a",
                    "description": "Tool A",
                    "inputSchema": {"type": "object", "properties": {}}
                },
                {
                    "name": "tool_b",
                    "inputSchema": {"type": "object", "properties": {}}
                }
            ]
        });
        let result: ToolsListResult =
            serde_json::from_value(raw).expect("deserialization should succeed");
        assert_eq!(result.tools.len(), 2);
        assert_eq!(result.tools[0].name, "tool_a");
        assert_eq!(result.tools[1].name, "tool_b");
    }

    #[test]
    fn deserialize_tool_call_result_text() {
        let raw = json!({
            "content": [
                {"type": "text", "text": "Hello world"}
            ]
        });
        let result: ToolCallResult =
            serde_json::from_value(raw).expect("deserialization should succeed");
        assert_eq!(result.content.len(), 1);
        assert!(!result.is_error);
        match &result.content[0] {
            ContentBlock::Text { text } => assert_eq!(text, "Hello world"),
            _ => panic!("expected text content block"),
        }
    }

    #[test]
    fn deserialize_tool_call_result_error() {
        let raw = json!({
            "content": [
                {"type": "text", "text": "something went wrong"}
            ],
            "is_error": true
        });
        let result: ToolCallResult =
            serde_json::from_value(raw).expect("deserialization should succeed");
        assert!(result.is_error);
    }

    #[test]
    fn deserialize_tool_call_result_image() {
        let raw = json!({
            "content": [
                {"type": "image", "data": "aWNvbg==", "mimeType": "image/png"}
            ]
        });
        let result: ToolCallResult =
            serde_json::from_value(raw).expect("deserialization should succeed");
        match &result.content[0] {
            ContentBlock::Image { data, mime_type } => {
                assert_eq!(data, "aWNvbg==");
                assert_eq!(mime_type, "image/png");
            }
            _ => panic!("expected image content block"),
        }
    }

    #[test]
    fn deserialize_tool_call_result_mixed_content() {
        let raw = json!({
            "content": [
                {"type": "text", "text": "Result:"},
                {"type": "text", "text": "42"}
            ]
        });
        let result: ToolCallResult =
            serde_json::from_value(raw).expect("deserialization should succeed");
        assert_eq!(result.content.len(), 2);
    }
}
