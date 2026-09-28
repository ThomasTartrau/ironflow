//! Integration tests for the MCP HTTP transport: tool filtering and
//! authenticated headers, against a wiremock MCP server.

#![cfg(feature = "tool-mcp")]

use serde_json::{Value, json};
use wiremock::matchers::{header, method};
use wiremock::{Match, Mock, MockServer, Request, ResponseTemplate};

use ironflow_core::providers::http::tools::ToolRegistry;
use ironflow_core::providers::http::tools::mcp::{
    McpConnection, McpError, McpToolFilter, register_mcp_tools, register_mcp_tools_filtered,
};

/// Matches a JSON-RPC POST body by its `method` field, ignoring `id`/`params`.
struct JsonRpcMethod(&'static str);

impl Match for JsonRpcMethod {
    fn matches(&self, request: &Request) -> bool {
        serde_json::from_slice::<Value>(&request.body)
            .ok()
            .and_then(|body| {
                body.get("method")
                    .and_then(|m| m.as_str())
                    .map(str::to_owned)
            })
            .as_deref()
            == Some(self.0)
    }
}

async fn mount_initialize(server: &MockServer) {
    Mock::given(method("POST"))
        .and(JsonRpcMethod("initialize"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "fake-mcp", "version": "0"}
            }
        })))
        .mount(server)
        .await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("notifications/initialized"))
        .respond_with(ResponseTemplate::new(200))
        .mount(server)
        .await;
}

fn tools_list_response(tools: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": 2, "result": {"tools": tools}})
}

#[tokio::test]
async fn filtered_registration_keeps_only_allowed_tool() {
    let server = MockServer::start().await;
    mount_initialize(&server).await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tools_list_response(json!([
            {"name": "read_status", "description": "read", "inputSchema": {"type": "object", "properties": {}}},
            {"name": "delete_everything", "description": "write", "inputSchema": {"type": "object", "properties": {}}}
        ]))))
        .mount(&server)
        .await;

    let conn = McpConnection::http(&server.uri())
        .await
        .expect("connection should build");
    let filter = McpToolFilter::allow(["read_status"]);

    let registry = register_mcp_tools_filtered(ToolRegistry::new(), conn, "srv", filter)
        .await
        .expect("registration should succeed");

    assert!(registry.has_tool("srv__read_status"));
    assert!(!registry.has_tool("srv__delete_everything"));
}

#[tokio::test]
async fn filtered_registration_errors_when_allowed_tool_missing() {
    let server = MockServer::start().await;
    mount_initialize(&server).await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tools_list_response(json!([
            {"name": "read_status", "description": "read", "inputSchema": {"type": "object", "properties": {}}}
        ]))))
        .mount(&server)
        .await;

    let conn = McpConnection::http(&server.uri())
        .await
        .expect("connection should build");
    let filter = McpToolFilter::allow(["zeta_missing", "read_status", "alpha_missing"]);

    let err = register_mcp_tools_filtered(ToolRegistry::new(), conn, "srv", filter)
        .await
        .expect_err("registration should fail on a missing allowed tool");

    // Every missing name is reported, sorted, so the message is stable.
    match err {
        McpError::ToolNotFound { names } => {
            assert_eq!(names, vec!["alpha_missing", "zeta_missing"]);
        }
        other => panic!("expected ToolNotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn filtered_registration_errors_when_allowed_tool_is_not_read_only() {
    let server = MockServer::start().await;
    mount_initialize(&server).await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tools_list_response(json!([
            {"name": "read_status", "description": "read", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}},
            {"name": "write_config", "description": "write", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": false}},
            {"name": "unmarked", "description": "unknown", "inputSchema": {"type": "object", "properties": {}}}
        ]))))
        .mount(&server)
        .await;

    let conn = McpConnection::http(&server.uri())
        .await
        .expect("connection should build");
    let filter =
        McpToolFilter::allow(["read_status", "write_config", "unmarked"]).require_read_only_hint();

    let err = register_mcp_tools_filtered(ToolRegistry::new(), conn, "srv", filter)
        .await
        .expect_err("a tool asked for by name must not be dropped silently");

    match err {
        McpError::ToolNotReadOnly { names } => {
            assert_eq!(names, vec!["unmarked", "write_config"]);
        }
        other => panic!("expected ToolNotReadOnly, got {other:?}"),
    }
}

#[tokio::test]
async fn filtered_registration_keeps_allowed_read_only_tools() {
    let server = MockServer::start().await;
    mount_initialize(&server).await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tools_list_response(json!([
            {"name": "read_status", "description": "read", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}},
            {"name": "read_logs", "description": "read", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}}
        ]))))
        .mount(&server)
        .await;

    let conn = McpConnection::http(&server.uri())
        .await
        .expect("connection should build");
    let filter = McpToolFilter::allow(["read_status"]).require_read_only_hint();

    let registry = register_mcp_tools_filtered(ToolRegistry::new(), conn, "srv", filter)
        .await
        .expect("registration should succeed");

    assert!(registry.has_tool("srv__read_status"));
    assert!(!registry.has_tool("srv__read_logs"));
}

#[tokio::test]
async fn filtered_registration_requires_read_only_hint() {
    let server = MockServer::start().await;
    mount_initialize(&server).await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tools_list_response(json!([
            {"name": "read_status", "description": "read", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": true}},
            {"name": "write_config", "description": "write", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": false}},
            {"name": "unmarked", "description": "unknown", "inputSchema": {"type": "object", "properties": {}}}
        ]))))
        .mount(&server)
        .await;

    let conn = McpConnection::http(&server.uri())
        .await
        .expect("connection should build");
    let filter = McpToolFilter::read_only();

    let registry = register_mcp_tools_filtered(ToolRegistry::new(), conn, "srv", filter)
        .await
        .expect("registration should succeed");

    assert!(registry.has_tool("srv__read_status"));
    assert!(!registry.has_tool("srv__write_config"));
    assert!(!registry.has_tool("srv__unmarked"));
}

#[tokio::test]
async fn read_only_filter_without_read_only_tool_registers_nothing() {
    let server = MockServer::start().await;
    mount_initialize(&server).await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tools_list_response(json!([
            {"name": "write_config", "description": "write", "inputSchema": {"type": "object", "properties": {}}, "annotations": {"readOnlyHint": false}}
        ]))))
        .mount(&server)
        .await;

    let conn = McpConnection::http(&server.uri())
        .await
        .expect("connection should build");

    let registry =
        register_mcp_tools_filtered(ToolRegistry::new(), conn, "srv", McpToolFilter::read_only())
            .await
            .expect("registration should succeed");

    assert!(registry.is_empty());
}

#[tokio::test]
async fn unfiltered_registration_keeps_previous_behavior() {
    let server = MockServer::start().await;
    mount_initialize(&server).await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("tools/list"))
        .respond_with(ResponseTemplate::new(200).set_body_json(tools_list_response(json!([
            {"name": "read_status", "description": "read", "inputSchema": {"type": "object", "properties": {}}},
            {"name": "delete_everything", "description": "write", "inputSchema": {"type": "object", "properties": {}}}
        ]))))
        .mount(&server)
        .await;

    let conn = McpConnection::http(&server.uri())
        .await
        .expect("connection should build");

    let registry = register_mcp_tools(ToolRegistry::new(), conn, "srv")
        .await
        .expect("registration should succeed");

    assert!(registry.has_tool("srv__read_status"));
    assert!(registry.has_tool("srv__delete_everything"));
}

#[tokio::test]
async fn http_transport_sends_configured_headers() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("initialize"))
        .and(header("Authorization", "Bearer secret-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "serverInfo": {"name": "fake-mcp", "version": "0"}
            }
        })))
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(JsonRpcMethod("notifications/initialized"))
        .and(header("Authorization", "Bearer secret-token"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let mut conn = McpConnection::http_with_headers(
        &server.uri(),
        &[("Authorization", "Bearer secret-token")],
    )
    .await
    .expect("connection should build");

    conn.initialize()
        .await
        .expect("initialize should succeed against an authenticated server");
}

#[tokio::test]
async fn http_connection_debug_omits_header_values() {
    let conn = McpConnection::http_with_headers(
        "http://127.0.0.1:0",
        &[("Authorization", "Bearer super-secret-token")],
    )
    .await
    .expect("connection should build");

    let debug = format!("{conn:?}");
    assert!(debug.contains("Authorization"));
    assert!(!debug.contains("super-secret-token"));
}

#[tokio::test]
async fn http_with_headers_rejects_duplicate_header_names() {
    let err = McpConnection::http_with_headers(
        "http://127.0.0.1:0",
        &[
            ("Authorization", "Bearer first-secret"),
            ("authorization", "Bearer second-secret"),
        ],
    )
    .await
    .expect_err("header names are case-insensitive, so this is a duplicate");

    let message = err.to_string();
    assert!(matches!(err, McpError::ConnectionFailed { .. }));
    assert!(message.contains("more than once"));
    assert!(!message.contains("first-secret"));
    assert!(!message.contains("second-secret"));
}

#[tokio::test]
async fn http_with_headers_rejects_invalid_value_without_leaking_it() {
    let err = McpConnection::http_with_headers(
        "http://127.0.0.1:0",
        &[("Authorization", "Bearer secret\nwith-newline")],
    )
    .await
    .expect_err("a newline is not a valid header value");

    let message = err.to_string();
    assert!(matches!(err, McpError::ConnectionFailed { .. }));
    assert!(message.contains("Authorization"));
    assert!(!message.contains("secret"));
}
