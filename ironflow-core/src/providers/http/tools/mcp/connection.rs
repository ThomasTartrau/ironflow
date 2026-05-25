//! MCP server connection management (stdio and HTTP transports).

use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, warn};

use super::error::McpError;
use super::protocol::{
    ContentBlock, JsonRpcNotification, JsonRpcRequest, JsonRpcResponse, ToolCallResult,
    ToolsListResult,
};

const MAX_RESPONSE_SIZE: usize = 100 * 1024;
const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Connection to an MCP server.
///
/// Supports stdio transport (subprocess communication) and HTTP transport
/// (simple POST-based JSON-RPC). Each connection manages the JSON-RPC protocol
/// lifecycle including initialization handshake.
///
/// # Examples
///
/// ```no_run
/// use ironflow_core::providers::http::tools::mcp::McpConnection;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let mut conn = McpConnection::stdio(
///     "mcp-grafana",
///     &["stdio"],
///     &[("GRAFANA_URL", "http://localhost:3000")],
/// ).await?;
/// conn.initialize().await?;
/// let tools = conn.list_tools().await?;
/// # Ok(())
/// # }
/// ```
pub enum McpConnection {
    /// Stdio transport (subprocess).
    #[doc(hidden)]
    Stdio(StdioTransport),
    /// HTTP transport (POST-based JSON-RPC).
    #[doc(hidden)]
    Http(HttpTransport),
}

/// Stdio transport (detail d'implementation).
#[doc(hidden)]
pub struct StdioTransport {
    child: Child,
    writer: Mutex<BufWriter<ChildStdin>>,
    reader: Mutex<BufReader<ChildStdout>>,
    next_id: AtomicU64,
}

/// HTTP transport (detail d'implementation).
#[doc(hidden)]
pub struct HttpTransport {
    client: reqwest::Client,
    base_url: String,
    next_id: AtomicU64,
}

impl McpConnection {
    /// Connect to an MCP server via stdio (subprocess).
    ///
    /// Spawns the given command with the specified arguments and environment variables.
    /// Communication happens via newline-delimited JSON-RPC on stdin/stdout.
    ///
    /// # Errors
    ///
    /// Returns [`McpError::SpawnFailed`] if the subprocess cannot be started.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::http::tools::mcp::McpConnection;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let conn = McpConnection::stdio("mcp-server", &["--mode", "stdio"], &[]).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn stdio(
        command: &str,
        args: &[&str],
        env: &[(&str, &str)],
    ) -> Result<Self, McpError> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        for (key, val) in env {
            cmd.env(key, val);
        }

        let mut child = cmd.spawn().map_err(|e| McpError::SpawnFailed {
            command: command.to_string(),
            reason: e.to_string(),
        })?;

        let stdin = child.stdin.take().ok_or_else(|| McpError::SpawnFailed {
            command: command.to_string(),
            reason: "failed to capture stdin".to_string(),
        })?;

        let stdout = child.stdout.take().ok_or_else(|| McpError::SpawnFailed {
            command: command.to_string(),
            reason: "failed to capture stdout".to_string(),
        })?;

        Ok(Self::Stdio(StdioTransport {
            child,
            writer: Mutex::new(BufWriter::new(stdin)),
            reader: Mutex::new(BufReader::new(stdout)),
            next_id: AtomicU64::new(1),
        }))
    }

    /// Connect to an MCP server via HTTP (POST-based JSON-RPC).
    ///
    /// # Errors
    ///
    /// Returns [`McpError::ConnectionFailed`] if the HTTP client cannot be built.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_core::providers::http::tools::mcp::McpConnection;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let conn = McpConnection::http("http://localhost:8080/mcp").await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn http(base_url: &str) -> Result<Self, McpError> {
        let client = reqwest::Client::builder()
            .timeout(CALL_TIMEOUT)
            .build()
            .map_err(|e| McpError::ConnectionFailed {
                url: base_url.to_string(),
                reason: e.to_string(),
            })?;

        Ok(Self::Http(HttpTransport {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            next_id: AtomicU64::new(1),
        }))
    }

    /// Perform the MCP initialization handshake.
    ///
    /// Sends `initialize` request and `notifications/initialized` notification
    /// as required by the MCP specification.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if the handshake fails.
    pub async fn initialize(&mut self) -> Result<(), McpError> {
        let params = json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "ironflow",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let response = self.send_request("initialize", Some(params)).await?;

        if let Some(err) = response.error {
            return Err(McpError::ProtocolError {
                message: format!("initialize failed: {} (code {})", err.message, err.code),
            });
        }

        debug!("MCP server initialized successfully");

        self.send_notification("notifications/initialized", None)
            .await?;

        Ok(())
    }

    /// Discover available tools from the MCP server.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if the request fails or the response is malformed.
    pub async fn list_tools(&self) -> Result<Vec<super::protocol::McpToolDef>, McpError> {
        let response = self.send_request("tools/list", None).await?;

        if let Some(err) = response.error {
            return Err(McpError::ProtocolError {
                message: format!("tools/list failed: {} (code {})", err.message, err.code),
            });
        }

        let result_value = response.result.ok_or_else(|| McpError::ProtocolError {
            message: "tools/list returned no result".to_string(),
        })?;

        let result: ToolsListResult =
            serde_json::from_value(result_value).map_err(|e| McpError::ProtocolError {
                message: format!("failed to parse tools/list result: {e}"),
            })?;

        Ok(result.tools)
    }

    /// Execute a tool on the MCP server.
    ///
    /// Returns the concatenated text content from the response, truncated
    /// to 100 KB if necessary.
    ///
    /// # Errors
    ///
    /// Returns [`McpError`] if the call fails or times out.
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<String, McpError> {
        let params = json!({
            "name": name,
            "arguments": arguments
        });

        let response = timeout(CALL_TIMEOUT, self.send_request("tools/call", Some(params)))
            .await
            .map_err(|_| McpError::Timeout {
                tool_name: name.to_string(),
            })??;

        if let Some(err) = response.error {
            return Err(McpError::ToolCallFailed {
                tool_name: name.to_string(),
                message: format!("{} (code {})", err.message, err.code),
            });
        }

        let result_value = response.result.ok_or_else(|| McpError::ToolCallFailed {
            tool_name: name.to_string(),
            message: "no result in response".to_string(),
        })?;

        let result: ToolCallResult =
            serde_json::from_value(result_value).map_err(|e| McpError::ProtocolError {
                message: format!("failed to parse tools/call result: {e}"),
            })?;

        if result.is_error {
            let error_text = extract_text_content(&result.content);
            return Err(McpError::ToolCallFailed {
                tool_name: name.to_string(),
                message: error_text,
            });
        }

        let mut output = extract_text_content(&result.content);

        if output.len() > MAX_RESPONSE_SIZE {
            output.truncate(MAX_RESPONSE_SIZE);
            output.push_str("\n... [truncated]");
        }

        Ok(output)
    }

    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, McpError> {
        match self {
            Self::Stdio(transport) => transport.send_request(method, params).await,
            Self::Http(transport) => transport.send_request(method, params).await,
        }
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        match self {
            Self::Stdio(transport) => transport.send_notification(method, params).await,
            Self::Http(transport) => transport.send_notification(method, params).await,
        }
    }
}

impl StdioTransport {
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest::new(id, method, params);

        let mut payload = serde_json::to_string(&request).map_err(|e| McpError::ProtocolError {
            message: format!("failed to serialize request: {e}"),
        })?;
        payload.push('\n');

        {
            let mut writer = self.writer.lock().await;
            writer
                .write_all(payload.as_bytes())
                .await
                .map_err(|e| McpError::IoError {
                    message: format!("failed to write to stdin: {e}"),
                })?;
            writer.flush().await.map_err(|e| McpError::IoError {
                message: format!("failed to flush stdin: {e}"),
            })?;
        }

        let mut line = String::new();
        {
            let mut reader = self.reader.lock().await;
            reader
                .read_line(&mut line)
                .await
                .map_err(|e| McpError::IoError {
                    message: format!("failed to read from stdout: {e}"),
                })?;
        }

        if line.is_empty() {
            return Err(McpError::IoError {
                message: "MCP server closed stdout unexpectedly".to_string(),
            });
        }

        serde_json::from_str(&line).map_err(|e| McpError::ProtocolError {
            message: format!("failed to parse response: {e}"),
        })
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let notification = JsonRpcNotification::new(method, params);

        let mut payload =
            serde_json::to_string(&notification).map_err(|e| McpError::ProtocolError {
                message: format!("failed to serialize notification: {e}"),
            })?;
        payload.push('\n');

        let mut writer = self.writer.lock().await;
        writer
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| McpError::IoError {
                message: format!("failed to write notification to stdin: {e}"),
            })?;
        writer.flush().await.map_err(|e| McpError::IoError {
            message: format!("failed to flush stdin: {e}"),
        })?;

        Ok(())
    }
}

impl HttpTransport {
    async fn send_request(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<JsonRpcResponse, McpError> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let request = JsonRpcRequest::new(id, method, params);

        let response = self
            .client
            .post(&self.base_url)
            .json(&request)
            .send()
            .await
            .map_err(|e| McpError::ConnectionFailed {
                url: self.base_url.clone(),
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            return Err(McpError::ConnectionFailed {
                url: self.base_url.clone(),
                reason: format!("HTTP {}", response.status()),
            });
        }

        let body = response.text().await.map_err(|e| McpError::IoError {
            message: format!("failed to read response body: {e}"),
        })?;

        serde_json::from_str(&body).map_err(|e| McpError::ProtocolError {
            message: format!("failed to parse response: {e}"),
        })
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<(), McpError> {
        let notification = JsonRpcNotification::new(method, params);

        let response = self
            .client
            .post(&self.base_url)
            .json(&notification)
            .send()
            .await
            .map_err(|e| McpError::ConnectionFailed {
                url: self.base_url.clone(),
                reason: e.to_string(),
            })?;

        if !response.status().is_success() {
            warn!(
                method = method,
                status = %response.status(),
                "MCP notification returned non-success status"
            );
        }

        Ok(())
    }
}

impl Drop for McpConnection {
    fn drop(&mut self) {
        if let Self::Stdio(transport) = self {
            let _ = transport.child.start_kill();
        }
    }
}

fn extract_text_content(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            ContentBlock::Image { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
