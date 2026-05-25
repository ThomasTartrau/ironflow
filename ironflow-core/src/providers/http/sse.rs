//! Generic SSE (Server-Sent Events) stream parser for HTTP provider responses.

use std::time::{Duration, Instant};

use reqwest::Response;
use serde_json::Value;

use crate::error::AgentError;
use crate::providers::http::adapter::HttpAgentAdapter;

/// A single parsed SSE event delta.
#[derive(Debug, Clone)]
pub enum SseDelta {
    /// Text content fragment.
    Text(String),
    /// Tool call fragment (may arrive incrementally).
    ToolCallDelta {
        /// Index of the tool call in the array.
        index: usize,
        /// Call identifier (only on first fragment).
        id: Option<String>,
        /// Tool name (only on first fragment).
        name: Option<String>,
        /// Incremental JSON arguments fragment.
        args_fragment: String,
    },
    /// Token usage reported at stream end.
    Usage {
        /// Input tokens consumed.
        input_tokens: u64,
        /// Output tokens generated.
        output_tokens: u64,
    },
    /// Structured output value extracted from a tool_use block.
    StructuredValue(Value),
    /// Stream is complete.
    Done,
}

/// Collect all SSE deltas from a streaming HTTP response.
///
/// Reads the response body chunk by chunk, splits on newline boundaries,
/// and delegates each `data:` line to the adapter's [`parse_sse_line`](HttpAgentAdapter::parse_sse_line).
pub async fn collect_sse_stream<A: HttpAgentAdapter>(
    adapter: &A,
    response: Response,
    timeout: Duration,
) -> Result<Vec<SseDelta>, AgentError> {
    let mut deltas: Vec<SseDelta> = Vec::new();
    let mut buffer = String::new();
    let deadline = Instant::now() + timeout;

    let mut stream = response;

    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AgentError::Timeout { limit: timeout });
        }

        let chunk_result = tokio::time::timeout(remaining, stream.chunk()).await;

        match chunk_result {
            Ok(Ok(Some(chunk))) => {
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim_end_matches('\r').to_string();
                    buffer.drain(..=newline_pos);

                    if let Some(data) = line.strip_prefix("data: ")
                        && let Some(delta) = adapter.parse_sse_line(data)
                    {
                        let is_done = matches!(delta, SseDelta::Done);
                        deltas.push(delta);
                        if is_done {
                            return Ok(deltas);
                        }
                    }
                }
            }
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                return Err(AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("SSE stream read error: {e}"),
                });
            }
            Err(_) => {
                return Err(AgentError::Timeout { limit: timeout });
            }
        }
    }

    Ok(deltas)
}
