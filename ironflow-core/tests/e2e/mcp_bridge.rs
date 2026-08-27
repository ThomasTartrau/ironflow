//! Integration tests for the MCP bridge module.
//!
//! These tests spawn a minimal MCP server implemented as a bash script
//! to verify the full stdio transport lifecycle: initialize, list_tools, call_tool.

#![cfg(feature = "tool-mcp")]

use std::time::Duration;

use serde_json::json;
use tokio::time::timeout;

use ironflow_core::providers::http::tools::ToolRegistry;
use ironflow_core::providers::http::tools::mcp::{McpConnection, McpError, register_mcp_tools};

const ECHO_MCP_SERVER: &str = r#"#!/bin/bash
# Minimal MCP server that responds to initialize, tools/list, and tools/call
while IFS= read -r line; do
    method=$(echo "$line" | python3 -c "import sys,json; print(json.loads(sys.stdin.read())['method'])" 2>/dev/null)
    id=$(echo "$line" | python3 -c "import sys,json; print(json.loads(sys.stdin.read())['id'])" 2>/dev/null)

    case "$method" in
        "initialize")
            echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"serverInfo\":{\"name\":\"echo-mcp\",\"version\":\"1.0.0\"}}}"
            ;;
        "notifications/initialized")
            # notification, no response
            ;;
        "tools/list")
            echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"tools\":[{\"name\":\"echo\",\"description\":\"Echoes input back\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"message\":{\"type\":\"string\"}},\"required\":[\"message\"]}},{\"name\":\"add\",\"description\":\"Adds two numbers\",\"inputSchema\":{\"type\":\"object\",\"properties\":{\"a\":{\"type\":\"number\"},\"b\":{\"type\":\"number\"}},\"required\":[\"a\",\"b\"]}}]}}"
            ;;
        "tools/call")
            name=$(echo "$line" | python3 -c "import sys,json; print(json.loads(sys.stdin.read())['params']['name'])" 2>/dev/null)
            if [ "$name" = "echo" ]; then
                msg=$(echo "$line" | python3 -c "import sys,json; print(json.loads(sys.stdin.read())['params']['arguments']['message'])" 2>/dev/null)
                echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"$msg\"}]}}"
            elif [ "$name" = "add" ]; then
                result=$(echo "$line" | python3 -c "import sys,json; d=json.loads(sys.stdin.read())['params']['arguments']; print(d['a']+d['b'])" 2>/dev/null)
                echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"$result\"}]}}"
            else
                echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"unknown tool\"}],\"is_error\":true}}"
            fi
            ;;
        *)
            echo "{\"jsonrpc\":\"2.0\",\"id\":$id,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}"
            ;;
    esac
done
"#;

/// Spawns the echo MCP server by passing the script inline to `bash -c`.
///
/// This avoids writing a temporary file, which eliminates `ETXTBSY` races
/// that occur when the kernel has not fully released the file descriptor
/// before `exec` (especially under Docker + Rosetta emulation).
async fn spawn_echo_server() -> McpConnection {
    McpConnection::stdio("bash", &["-c", ECHO_MCP_SERVER], &[])
        .await
        .expect("failed to spawn echo MCP server")
}

#[tokio::test]
async fn initialize_and_list_tools() {
    timeout(Duration::from_secs(10), async {
        let mut conn = spawn_echo_server().await;
        conn.initialize().await.expect("initialize should succeed");

        let tools = conn.list_tools().await.expect("list_tools should succeed");
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "echo");
        assert_eq!(tools[1].name, "add");
        assert_eq!(tools[0].description.as_deref(), Some("Echoes input back"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn call_tool_echo() {
    timeout(Duration::from_secs(10), async {
        let mut conn = spawn_echo_server().await;
        conn.initialize().await.expect("initialize should succeed");

        let result = conn
            .call_tool("echo", json!({"message": "hello world"}))
            .await
            .expect("call_tool should succeed");

        assert_eq!(result, "hello world");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn call_tool_add() {
    timeout(Duration::from_secs(10), async {
        let mut conn = spawn_echo_server().await;
        conn.initialize().await.expect("initialize should succeed");

        let result = conn
            .call_tool("add", json!({"a": 3, "b": 4}))
            .await
            .expect("call_tool should succeed");

        assert_eq!(result, "7");
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn call_unknown_tool_returns_error() {
    timeout(Duration::from_secs(10), async {
        let mut conn = spawn_echo_server().await;
        conn.initialize().await.expect("initialize should succeed");

        let result = conn.call_tool("nonexistent", json!({})).await;
        assert!(matches!(result, Err(McpError::ToolCallFailed { .. })));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn register_mcp_tools_populates_registry() {
    timeout(Duration::from_secs(10), async {
        let conn = spawn_echo_server().await;
        let registry = ToolRegistry::new();

        let registry = register_mcp_tools(registry, conn, "test")
            .await
            .expect("register should succeed");

        assert_eq!(registry.len(), 2);
        assert!(registry.has_tool("test__echo"));
        assert!(registry.has_tool("test__add"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn bridge_tool_execute_via_registry() {
    timeout(Duration::from_secs(10), async {
        let conn = spawn_echo_server().await;
        let registry = register_mcp_tools(ToolRegistry::new(), conn, "srv")
            .await
            .expect("register should succeed");

        let result = registry
            .execute("srv__echo", json!({"message": "ping"}))
            .await
            .expect("tool should exist")
            .expect("execution should succeed");

        assert_eq!(result.content, "ping");
        assert!(!result.is_error);
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn register_mcp_tools_uses_double_underscore_separator() {
    timeout(Duration::from_secs(10), async {
        let conn = spawn_echo_server().await;
        let registry = register_mcp_tools(ToolRegistry::new(), conn, "myprefix")
            .await
            .expect("register should succeed");

        assert!(registry.has_tool("myprefix__echo"));
        assert!(registry.has_tool("myprefix__add"));
        assert!(!registry.has_tool("myprefix.echo"));
        assert!(!registry.has_tool("myprefix.add"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn register_mcp_tools_tracks_connector() {
    timeout(Duration::from_secs(10), async {
        let conn = spawn_echo_server().await;
        let registry = register_mcp_tools(ToolRegistry::new(), conn, "grafana")
            .await
            .expect("register should succeed");

        assert!(registry.connectors().contains("grafana"));
    })
    .await
    .expect("test timed out");
}

#[tokio::test]
async fn spawn_nonexistent_command_fails() {
    let result = McpConnection::stdio("nonexistent_mcp_binary_xyz", &[], &[]).await;
    assert!(matches!(result, Err(McpError::SpawnFailed { .. })));
}
