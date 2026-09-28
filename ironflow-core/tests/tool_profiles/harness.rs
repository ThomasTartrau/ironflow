//! Test harness: a fake OpenAI-compatible LLM server, a wire adapter for it,
//! and a counting tool.
//!
//! The server records every request body and answers with canned responses.
//! The adapter only translates the wire format; the profile selection, tool
//! injection and tool execution under test are the provider's own code.

use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, from_slice, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::spawn;
use tokio::time::timeout;

use ironflow_core::error::AgentError;
use ironflow_core::provider::{AgentConfig, LogSink, ToolProfile};
use ironflow_core::providers::http::HttpAgentProvider;
use ironflow_core::providers::http::adapter::{
    HttpAgentAdapter, HttpToolCall, HttpUsage, TurnResult,
};
use ironflow_core::providers::http::sse::SseDelta;
use ironflow_core::providers::http::tools::{Tool, ToolError, ToolOutput, ToolRegistry};

/// Profile for proposing a fix.
pub const SUGGESTION: ToolProfile = ToolProfile::new("suggestion");
/// Profile for investigating a bug.
pub const BUG: ToolProfile = ToolProfile::new("bug");
/// A profile no provider in these tests registers.
pub const INCIDENT: ToolProfile = ToolProfile::new("incident");

/// Request bodies received by the fake server, in arrival order.
pub type Requests = Arc<Mutex<Vec<Value>>>;

async fn read_body(stream: &mut TcpStream) -> Value {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        let n = stream.read(&mut chunk).await.expect("read request");
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf);
        if let Some(header_end) = text.find("\r\n\r\n") {
            let content_length = text[..header_end]
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().expect("content-length"))
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            if buf.len() >= body_start + content_length {
                return from_slice(&buf[body_start..body_start + content_length])
                    .expect("request body is JSON");
            }
        }
    }
    panic!("connection closed before the request body was read");
}

/// Serve `responses` in order, one per request, and record each request body.
pub async fn spawn_llm(responses: Vec<Value>) -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let requests: Requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let mut queue: VecDeque<Value> = responses.into();
    spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let body = read_body(&mut stream).await;
            recorded.lock().expect("lock").push(body);
            let reply = queue
                .pop_front()
                .expect("the provider sent more requests than expected")
                .to_string();
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        }
    });
    (format!("http://{addr}"), requests)
}

/// A final assistant answer carrying `text`.
pub fn final_answer(text: &str) -> Value {
    json!({
        "model": "fake-model",
        "choices": [{"message": {"role": "assistant", "content": text}, "finish_reason": "stop"}]
    })
}

/// An assistant turn that calls `tool` once.
pub fn tool_call_answer(tool: &str) -> Value {
    json!({
        "model": "fake-model",
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{"id": "call_1", "type": "function", "function": {"name": tool, "arguments": "{}"}}]
            },
            "finish_reason": "tool_calls"
        }]
    })
}

/// Names of the tools sent in a request body, or `None` without a `tools` key.
pub fn tool_names(request: &Value) -> Option<Vec<String>> {
    request.get("tools").map(|tools| {
        tools
            .as_array()
            .expect("tools is an array")
            .iter()
            .map(|t| {
                t["function"]["name"]
                    .as_str()
                    .expect("tool name")
                    .to_string()
            })
            .collect()
    })
}

/// Owned tool names, for readable assertions.
pub fn names(names: &[&str]) -> Option<Vec<String>> {
    Some(names.iter().map(|n| n.to_string()).collect())
}

/// Translates between [`AgentConfig`] and the fake server's OpenAI wire format.
pub struct FakeAdapter {
    base_url: String,
}

impl HttpAgentAdapter for FakeAdapter {
    fn provider_name(&self) -> &'static str {
        "fake"
    }

    fn endpoint_url(&self, _model: &str) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    fn build_request(&self, config: &AgentConfig) -> Result<Value, AgentError> {
        Ok(json!({
            "model": config.model,
            "messages": [{"role": "user", "content": config.prompt}]
        }))
    }

    fn parse_response(
        &self,
        body: &Value,
        _config: &AgentConfig,
    ) -> Result<TurnResult, AgentError> {
        let message = &body["choices"][0]["message"];
        let tool_calls: Vec<HttpToolCall> = message
            .get("tool_calls")
            .and_then(Value::as_array)
            .map(|calls| {
                calls
                    .iter()
                    .map(|c| HttpToolCall {
                        id: c["id"].as_str().expect("id").to_string(),
                        name: c["function"]["name"].as_str().expect("name").to_string(),
                        input: json!({}),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Ok(TurnResult {
            text: message["content"].as_str().map(String::from),
            is_final: tool_calls.is_empty(),
            tool_calls,
            structured_value: None,
            usage: HttpUsage::default(),
            model: body["model"].as_str().map(String::from),
        })
    }

    fn parse_sse_line(&self, _line: &str) -> Option<SseDelta> {
        None
    }

    fn fold_sse_deltas(
        &self,
        _deltas: Vec<SseDelta>,
        _config: &AgentConfig,
    ) -> Result<TurnResult, AgentError> {
        unreachable!("the tests never enable verbose streaming")
    }

    fn compute_cost(&self, _model: &str, _usage: &HttpUsage) -> Option<f64> {
        None
    }

    fn resolve_model(&self, model: &str) -> String {
        model.to_string()
    }
}

/// An HTTP provider talking to the fake server at `base_url`.
pub fn provider(base_url: &str) -> HttpAgentProvider<FakeAdapter> {
    HttpAgentProvider::new(FakeAdapter {
        base_url: base_url.to_string(),
    })
}

/// A tool that counts its executions.
pub struct CountingTool {
    name: &'static str,
    /// Number of times the tool ran.
    pub calls: Arc<AtomicUsize>,
}

impl CountingTool {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            calls: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl Tool for CountingTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        "Counts its calls"
    }

    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}})
    }

    fn execute(
        &self,
        _input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ToolOutput::success(format!("{} ran", self.name)))
        })
    }
}

/// A registry holding one [`CountingTool`] per name.
pub fn registry(names: &[&'static str]) -> ToolRegistry {
    names.iter().fold(ToolRegistry::new(), |reg, name| {
        reg.register(CountingTool::new(name))
    })
}

/// A [`LogSink`] that keeps every `(stream, line)` pair.
pub struct VecSink(pub Mutex<Vec<(String, String)>>);

impl VecSink {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(Mutex::new(Vec::new())))
    }

    pub fn lines(&self) -> Vec<(String, String)> {
        self.0.lock().expect("lock").clone()
    }
}

impl LogSink for VecSink {
    fn log(&self, stream: &str, line: &str) {
        self.0
            .lock()
            .expect("lock")
            .push((stream.to_string(), line.to_string()));
    }
}

/// Run `test` under a 10 second timeout.
pub async fn within_timeout<F: Future<Output = ()>>(test: F) {
    timeout(Duration::from_secs(10), test)
        .await
        .expect("test timed out");
}
