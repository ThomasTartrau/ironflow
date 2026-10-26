//! Test harness: a fake LLM server that records requests, and a counting tool.

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

use ironflow_core::providers::http::tools::{Tool, ToolError, ToolOutput, ToolRegistry};

/// Request bodies received by the fake server, in arrival order.
pub type Requests = Arc<Mutex<Vec<Value>>>;

/// A canned response of the fake server.
pub enum Reply {
    /// A JSON body.
    Json(Value),
    /// A raw server-sent events body.
    #[cfg_attr(not(feature = "provider-anthropic-api"), allow(dead_code))]
    Sse(String),
}

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
pub async fn spawn_llm(responses: Vec<Reply>) -> (String, Requests) {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let requests: Requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let mut queue: VecDeque<Reply> = responses.into();
    spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let body = read_body(&mut stream).await;
            recorded.lock().expect("lock").push(body);
            let (content_type, reply) = match queue
                .pop_front()
                .expect("the provider sent more requests than expected")
            {
                Reply::Json(value) => ("application/json", value.to_string()),
                Reply::Sse(text) => ("text/event-stream", text),
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
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

/// A registry holding `tool`, and the counter of its executions.
pub fn registry_with(tool: CountingTool) -> (ToolRegistry, Arc<AtomicUsize>) {
    let calls = tool.calls.clone();
    (ToolRegistry::new().register(tool), calls)
}

/// Run `test` under a 10 second timeout.
pub async fn within_timeout<F: Future<Output = ()>>(test: F) {
    timeout(Duration::from_secs(10), test)
        .await
        .expect("test timed out");
}
