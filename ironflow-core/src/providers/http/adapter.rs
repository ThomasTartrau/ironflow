//! Core adapter trait and generic provider wrapper for HTTP-based LLM APIs.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::future::join_all;
use reqwest::Client;
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};

use crate::error::AgentError;
use crate::provider::{
    AgentConfig, AgentOutput, AgentProvider, DebugMessage, DebugToolCall, DebugToolResult,
    InvokeFuture,
};
use crate::providers::http::sse::{SseDelta, collect_sse_stream};
use crate::providers::http::tools::ToolRegistry;
use crate::providers::http::tools::routing::route_tool_call;

/// Normalized result of one API turn (one HTTP request/response cycle).
#[derive(Debug)]
pub struct TurnResult {
    /// Free-form text content from the model.
    pub text: Option<String>,
    /// Tool calls requested by the model in this turn (unused in V1 - no tool execution).
    #[allow(dead_code)]
    pub tool_calls: Vec<HttpToolCall>,
    /// Whether this is the final turn.
    pub is_final: bool,
    /// Extracted structured JSON value when a schema was requested.
    pub structured_value: Option<Value>,
    /// Token usage reported by the provider.
    pub usage: HttpUsage,
    /// Concrete model identifier returned by the provider.
    pub model: Option<String>,
}

/// A single tool call requested by the model.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct HttpToolCall {
    /// Provider-assigned call identifier.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Input arguments as JSON.
    pub input: Value,
}

/// Token usage from a single turn.
#[derive(Debug, Default)]
pub struct HttpUsage {
    /// Uncached input/prompt tokens consumed (excludes cache reads and writes).
    pub input_tokens: Option<u64>,
    /// Input tokens served from the prompt cache.
    pub cache_read_input_tokens: Option<u64>,
    /// Input tokens written to the prompt cache.
    pub cache_creation_input_tokens: Option<u64>,
    /// Output/completion tokens generated.
    pub output_tokens: Option<u64>,
}

/// Internal trait implemented by each HTTP provider backend.
///
/// The generic [`HttpAgentProvider`] calls these methods to build requests,
/// parse responses, and configure authentication. The agentic loop, retry,
/// and timeout are handled by the wrapper.
pub trait HttpAgentAdapter: Send + Sync + 'static {
    /// Provider name for logging and errors.
    fn provider_name(&self) -> &'static str;

    /// Full endpoint URL for the given model.
    fn endpoint_url(&self, model: &str) -> String;

    /// Authentication and provider-specific headers.
    fn auth_headers(&self) -> Vec<(String, String)>;

    /// Build the initial JSON request body from an [`AgentConfig`].
    fn build_request(&self, config: &AgentConfig) -> Result<Value, AgentError>;

    /// Parse a non-streaming response body into a [`TurnResult`].
    fn parse_response(&self, body: &Value, config: &AgentConfig) -> Result<TurnResult, AgentError>;

    /// Parse a single SSE `data:` line into a streaming delta.
    fn parse_sse_line(&self, line: &str) -> Option<SseDelta>;

    /// Fold accumulated SSE deltas into a complete [`TurnResult`].
    fn fold_sse_deltas(
        &self,
        deltas: Vec<SseDelta>,
        config: &AgentConfig,
    ) -> Result<TurnResult, AgentError>;

    /// Compute cost in USD from the token usage of one turn, pricing
    /// uncached input, cache reads, cache writes and output separately.
    /// Returns `None` if unknown.
    fn compute_cost(&self, model: &str, usage: &HttpUsage) -> Option<f64>;

    /// Resolve model alias (e.g. "sonnet") to a provider-specific model ID.
    fn resolve_model(&self, model: &str) -> String;
}

/// Default timeout for HTTP provider requests.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Generic HTTP provider that wraps any [`HttpAgentAdapter`].
///
/// Implements [`AgentProvider`] by delegating request construction and response
/// parsing to the adapter while handling the HTTP transport, timeout, and
/// agentic execution loop.
///
/// When a [`ToolRegistry`] is attached via [`with_tools`](Self::with_tools),
/// the provider runs a multi-turn agentic loop: executing tool calls locally
/// and feeding results back to the model until it produces a final response
/// (or hits `max_turns` / `max_budget_usd` limits).
///
/// Without a registry, the provider behaves as single-turn (backward-compatible).
pub struct HttpAgentProvider<A: HttpAgentAdapter> {
    adapter: A,
    client: Client,
    timeout: Duration,
    tool_registry: Option<ToolRegistry>,
}

impl<A: HttpAgentAdapter> HttpAgentProvider<A> {
    /// Create a new HTTP provider with the given adapter.
    pub fn new(adapter: A) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("failed to build reqwest client");
        Self {
            adapter,
            client,
            timeout: DEFAULT_TIMEOUT,
            tool_registry: None,
        }
    }

    /// Attach a tool registry to enable multi-turn agentic execution.
    ///
    /// When tools are registered, the provider will:
    /// 1. Include the tools in every request (OpenAI `tools` format).
    /// 2. Execute tool calls returned by the model.
    /// 3. Loop until the model produces a final response or limits are hit.
    pub fn with_tools(mut self, registry: ToolRegistry) -> Self {
        self.tool_registry = Some(registry);
        self
    }

    /// Override the request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self.client = Client::builder()
            .timeout(timeout)
            .build()
            .expect("failed to build reqwest client");
        self
    }

    async fn execute_turn(
        &self,
        request_body: &Value,
        config: &AgentConfig,
    ) -> Result<TurnResult, AgentError> {
        let model = self.adapter.resolve_model(&config.model);
        let url = self.adapter.endpoint_url(&model);
        let headers = self.adapter.auth_headers();

        let mut req = self.client.post(&url).json(request_body);
        for (key, value) in &headers {
            req = req.header(key, value);
        }
        if let Some(ref ctx) = config.trace_context {
            req = req.header("traceparent", ctx.to_traceparent());
        }

        let response = tokio::time::timeout(self.timeout, req.send())
            .await
            .map_err(|_| AgentError::Timeout {
                limit: self.timeout,
            })?
            .map_err(|e| {
                if e.is_timeout() {
                    AgentError::Timeout {
                        limit: self.timeout,
                    }
                } else {
                    AgentError::HttpProvider {
                        provider: self.adapter.provider_name().to_string(),
                        status_code: 0,
                        message: format!("connection failed: {e}"),
                    }
                }
            })?;

        let status = response.status().as_u16();

        if status == 429 {
            let retry_after = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok());
            return Err(AgentError::RateLimited {
                provider: self.adapter.provider_name().to_string(),
                retry_after_secs: retry_after,
            });
        }

        if status >= 400 {
            let body_text = response.text().await.unwrap_or_default();
            let message = serde_json::from_str::<Value>(&body_text)
                .ok()
                .and_then(|v| {
                    v.get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|m| m.as_str())
                        .map(String::from)
                })
                .unwrap_or(body_text);
            return Err(AgentError::HttpProvider {
                provider: self.adapter.provider_name().to_string(),
                status_code: status,
                message,
            });
        }

        if config.verbose {
            let deltas = collect_sse_stream(&self.adapter, response, self.timeout).await?;
            self.adapter.fold_sse_deltas(deltas, config)
        } else {
            let body: Value = response
                .json()
                .await
                .map_err(|e| AgentError::HttpProvider {
                    provider: self.adapter.provider_name().to_string(),
                    status_code: 0,
                    message: format!("failed to parse response JSON: {e}"),
                })?;
            self.adapter.parse_response(&body, config)
        }
    }
}

/// Accumulates usage across turns and builds the final [`AgentOutput`].
struct LoopState {
    start: Instant,
    total_input_tokens: u64,
    total_cache_read_tokens: Option<u64>,
    total_cache_creation_tokens: Option<u64>,
    total_output_tokens: u64,
    total_cost: f64,
    model_name: Option<String>,
    debug_messages: Vec<DebugMessage>,
    verbose: bool,
}

impl LoopState {
    fn new(start: Instant, verbose: bool) -> Self {
        Self {
            start,
            total_input_tokens: 0,
            total_cache_read_tokens: None,
            total_cache_creation_tokens: None,
            total_output_tokens: 0,
            total_cost: 0.0,
            model_name: None,
            debug_messages: Vec::new(),
            verbose,
        }
    }

    fn into_output(self, value: Value) -> AgentOutput {
        AgentOutput {
            value,
            session_id: None,
            cost_usd: if self.total_cost > 0.0 {
                Some(self.total_cost)
            } else {
                None
            },
            input_tokens: Some(self.total_input_tokens),
            cache_read_input_tokens: self.total_cache_read_tokens,
            cache_creation_input_tokens: self.total_cache_creation_tokens,
            output_tokens: Some(self.total_output_tokens),
            model: self.model_name,
            duration_ms: self.start.elapsed().as_millis() as u64,
            debug_messages: if self.verbose {
                Some(self.debug_messages)
            } else {
                None
            },
        }
    }
}

/// Extract the final value from a turn result (structured or text).
fn extract_value(turn_result: &TurnResult) -> Value {
    if let Some(ref structured) = turn_result.structured_value {
        structured.clone()
    } else {
        turn_result
            .text
            .as_ref()
            .map(|t| Value::String(t.clone()))
            .unwrap_or(Value::String(String::new()))
    }
}

/// Extract the text value from a turn result (ignoring structured).
fn extract_text_value(turn_result: &TurnResult) -> Value {
    turn_result
        .text
        .as_ref()
        .map(|t| Value::String(t.clone()))
        .unwrap_or(Value::String(String::new()))
}

/// Execute a single tool call and return its `(content, is_error)` result.
///
/// Mirrors the model's routing rules: MCP-prefixed names are resolved
/// through `route_tool_call` when connectors are registered, otherwise the
/// name is looked up directly. An unknown tool or a routing failure is
/// reported back to the model as an error, never aborts the turn.
async fn execute_tool_call(
    tc: &HttpToolCall,
    registry: &ToolRegistry,
    provider_name: &'static str,
) -> (String, bool) {
    debug!(
        provider = provider_name,
        tool = %tc.name,
        call_id = %tc.id,
        "executing tool call"
    );

    let connectors = registry.connectors();
    let registry_key = if connectors.is_empty() {
        tc.name.clone()
    } else {
        match route_tool_call(&tc.name, connectors) {
            Ok(routed) => routed.registry_key,
            Err(routing_err) => return (routing_err.to_string(), true),
        }
    };

    match registry.execute(&registry_key, tc.input.clone()).await {
        Some(Ok(output)) => (output.content, output.is_error),
        Some(Err(err)) => (format!("Tool execution error: {err}"), true),
        None => (format!("Unknown tool: {}", tc.name), true),
    }
}

/// Execute every tool call of one turn.
///
/// Calls are processed in the model's order. A maximal run of consecutive
/// read-only calls (see [`Tool::read_only`](crate::providers::http::tools::Tool::read_only))
/// is executed concurrently via `join_all`, bounded by `max_parallel` permits
/// on a [`Semaphore`]. A non-read-only call is a barrier: it waits for the
/// previous group to finish, runs alone, and the next group only starts once
/// it completes. An unknown tool name (not present in the registry) is
/// treated as non-read-only.
///
/// Returns `(content, is_error)` pairs in the same order as `tool_calls`,
/// regardless of completion order within a parallel group.
///
/// # Panics
///
/// Panics if the internal semaphore is closed, which cannot happen since it
/// is never closed.
async fn execute_turn_tool_calls(
    tool_calls: &[HttpToolCall],
    registry: &ToolRegistry,
    provider_name: &'static str,
    max_parallel: usize,
) -> Vec<(String, bool)> {
    let max_parallel = max_parallel.max(1);
    let mut results = Vec::with_capacity(tool_calls.len());
    let mut idx = 0;

    while idx < tool_calls.len() {
        if registry.is_read_only(&tool_calls[idx].name) {
            let end = tool_calls[idx..]
                .iter()
                .position(|tc| !registry.is_read_only(&tc.name))
                .map(|offset| idx + offset)
                .unwrap_or(tool_calls.len());

            let semaphore = Arc::new(Semaphore::new(max_parallel));
            let group_results = join_all(tool_calls[idx..end].iter().map(|tc| {
                let semaphore = Arc::clone(&semaphore);
                async move {
                    let _permit = semaphore
                        .acquire()
                        .await
                        .expect("semaphore closed unexpectedly");
                    execute_tool_call(tc, registry, provider_name).await
                }
            }))
            .await;

            results.extend(group_results);
            idx = end;
        } else {
            results.push(execute_tool_call(&tool_calls[idx], registry, provider_name).await);
            idx += 1;
        }
    }

    results
}

impl<A: HttpAgentAdapter> AgentProvider for HttpAgentProvider<A> {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            let mut request_body = self.adapter.build_request(config)?;

            // Inject tools into the request if a registry is available
            if let Some(ref registry) = self.tool_registry
                && !registry.is_empty()
            {
                let tools_array = registry.to_openai_tools();
                request_body["tools"] = Value::Array(tools_array);
            }

            let max_turns = config.max_turns.unwrap_or(25) as usize;
            let max_budget = config.max_budget_usd.unwrap_or(f64::MAX);
            let mut state = LoopState::new(Instant::now(), config.verbose);

            // Messages array for multi-turn
            let mut messages: Vec<Value> = request_body
                .get("messages")
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_default();

            for turn in 0..max_turns {
                request_body["messages"] = Value::Array(messages.clone());
                let turn_result = self.execute_turn(&request_body, config).await?;

                // Accumulate usage
                let turn_input = turn_result.usage.input_tokens.unwrap_or(0);
                let turn_output = turn_result.usage.output_tokens.unwrap_or(0);
                state.total_input_tokens += turn_input;
                state.total_output_tokens += turn_output;
                if let Some(v) = turn_result.usage.cache_read_input_tokens {
                    state.total_cache_read_tokens =
                        Some(state.total_cache_read_tokens.unwrap_or(0) + v);
                }
                if let Some(v) = turn_result.usage.cache_creation_input_tokens {
                    state.total_cache_creation_tokens =
                        Some(state.total_cache_creation_tokens.unwrap_or(0) + v);
                }

                if state.model_name.is_none() {
                    state.model_name = turn_result.model.clone();
                }

                if let Some(ref model) = state.model_name
                    && let Some(turn_cost) = self.adapter.compute_cost(model, &turn_result.usage)
                {
                    state.total_cost += turn_cost;
                }

                // Record debug trace for this turn
                if config.verbose {
                    let tool_calls_debug: Vec<DebugToolCall> = turn_result
                        .tool_calls
                        .iter()
                        .map(|tc| DebugToolCall {
                            id: Some(tc.id.clone()),
                            name: tc.name.clone(),
                            input: tc.input.clone(),
                        })
                        .collect();

                    state.debug_messages.push(DebugMessage {
                        text: turn_result.text.clone(),
                        thinking: None,
                        thinking_redacted: false,
                        tool_calls: tool_calls_debug,
                        tool_results: Vec::new(),
                        stop_reason: if turn_result.is_final {
                            Some("end_turn".to_string())
                        } else {
                            Some("tool_use".to_string())
                        },
                        input_tokens: Some(turn_input),
                        output_tokens: Some(turn_output),
                    });
                }

                // Final response (no tool calls) -> return
                if turn_result.is_final || turn_result.tool_calls.is_empty() {
                    info!(
                        provider = self.adapter.provider_name(),
                        turns = turn + 1,
                        duration_ms = state.start.elapsed().as_millis() as u64,
                        input_tokens = state.total_input_tokens,
                        cache_read_input_tokens = state.total_cache_read_tokens,
                        cache_creation_input_tokens = state.total_cache_creation_tokens,
                        output_tokens = state.total_output_tokens,
                        "invocation complete"
                    );
                    return Ok(state.into_output(extract_value(&turn_result)));
                }

                // Tool calls but no registry -> return text (backward compat)
                let registry = match self.tool_registry {
                    Some(ref r) => r,
                    None => {
                        warn!(
                            provider = self.adapter.provider_name(),
                            tool_calls = turn_result.tool_calls.len(),
                            "model requested tool calls but no registry attached, returning text"
                        );
                        return Ok(state.into_output(extract_text_value(&turn_result)));
                    }
                };

                // Budget exceeded -> stop
                if state.total_cost >= max_budget {
                    warn!(
                        provider = self.adapter.provider_name(),
                        cost = state.total_cost,
                        budget = max_budget,
                        "budget exceeded, stopping agentic loop"
                    );
                    return Ok(state.into_output(extract_text_value(&turn_result)));
                }

                // Build assistant message with tool_calls for conversation history
                let assistant_tool_calls: Vec<Value> = turn_result
                    .tool_calls
                    .iter()
                    .map(|tc| {
                        json!({
                            "id": tc.id,
                            "type": "function",
                            "function": {
                                "name": tc.name,
                                "arguments": tc.input.to_string()
                            }
                        })
                    })
                    .collect();

                let mut assistant_msg = json!({"role": "assistant"});
                if let Some(ref text) = turn_result.text {
                    assistant_msg["content"] = Value::String(text.clone());
                } else {
                    assistant_msg["content"] = Value::Null;
                }
                assistant_msg["tool_calls"] = Value::Array(assistant_tool_calls);
                messages.push(assistant_msg);

                // Execute tool calls (consecutive read-only calls run concurrently)
                let max_parallel = config.max_parallel_tools.max(1);
                let results = execute_turn_tool_calls(
                    &turn_result.tool_calls,
                    registry,
                    self.adapter.provider_name(),
                    max_parallel,
                )
                .await;

                let mut tool_results_debug: Vec<DebugToolResult> = Vec::new();
                for (tc, (content, is_error)) in turn_result.tool_calls.iter().zip(results) {
                    messages.push(json!({
                        "role": "tool",
                        "tool_call_id": tc.id,
                        "content": content
                    }));

                    if config.verbose {
                        tool_results_debug.push(DebugToolResult {
                            tool_use_id: Some(tc.id.clone()),
                            content: Value::String(content.clone()),
                            is_error,
                        });
                    }
                }

                if config.verbose
                    && let Some(last_msg) = state.debug_messages.last_mut()
                {
                    last_msg.tool_results = tool_results_debug;
                }

                info!(
                    provider = self.adapter.provider_name(),
                    turn = turn + 1,
                    tools_executed = turn_result.tool_calls.len(),
                    "turn complete, continuing loop"
                );
            }

            warn!(
                provider = self.adapter.provider_name(),
                max_turns, "max turns reached, returning last state"
            );
            Ok(state.into_output(Value::String(String::new())))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use tokio::time::sleep;

    use crate::providers::http::tools::{Tool, ToolError, ToolOutput};

    use super::*;

    type CallLog = Arc<Mutex<Vec<(String, Instant, Instant)>>>;

    struct DelayTool {
        name: String,
        read_only: bool,
        delay: Duration,
        concurrent: Arc<AtomicUsize>,
        max_concurrent: Arc<AtomicUsize>,
        log: Option<CallLog>,
    }

    impl DelayTool {
        fn new(name: &str, read_only: bool, delay_ms: u64) -> Self {
            Self {
                name: name.to_string(),
                read_only,
                delay: Duration::from_millis(delay_ms),
                concurrent: Arc::new(AtomicUsize::new(0)),
                max_concurrent: Arc::new(AtomicUsize::new(0)),
                log: None,
            }
        }

        fn with_counters(mut self, concurrent: &Arc<AtomicUsize>, max: &Arc<AtomicUsize>) -> Self {
            self.concurrent = Arc::clone(concurrent);
            self.max_concurrent = Arc::clone(max);
            self
        }

        fn with_log(mut self, log: &CallLog) -> Self {
            self.log = Some(Arc::clone(log));
            self
        }
    }

    impl Tool for DelayTool {
        fn name(&self) -> &str {
            &self.name
        }

        fn description(&self) -> &str {
            "Sleeps then returns its name"
        }

        fn parameters_schema(&self) -> Value {
            json!({"type": "object", "properties": {}})
        }

        fn read_only(&self) -> bool {
            self.read_only
        }

        fn execute(
            &self,
            _input: Value,
        ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
            Box::pin(async move {
                let start = Instant::now();
                let now = self.concurrent.fetch_add(1, Ordering::SeqCst) + 1;
                self.max_concurrent.fetch_max(now, Ordering::SeqCst);
                sleep(self.delay).await;
                self.concurrent.fetch_sub(1, Ordering::SeqCst);
                let end = Instant::now();
                if let Some(ref log) = self.log {
                    log.lock()
                        .expect("log mutex poisoned")
                        .push((self.name.clone(), start, end));
                }
                Ok(ToolOutput::success(self.name.clone()))
            })
        }
    }

    fn call(id: &str, name: &str) -> HttpToolCall {
        HttpToolCall {
            id: id.to_string(),
            name: name.to_string(),
            input: json!({}),
        }
    }

    fn entry(log: &CallLog, name: &str) -> (Instant, Instant) {
        let entries = log.lock().expect("log mutex poisoned");
        let (_, start, end) = entries
            .iter()
            .find(|(n, _, _)| n == name)
            .unwrap_or_else(|| panic!("no log entry for {name}"));
        (*start, *end)
    }

    #[tokio::test]
    async fn parallel_read_only_tools() {
        let registry = ToolRegistry::new()
            .register(DelayTool::new("read_a", true, 200))
            .register(DelayTool::new("read_b", true, 200));
        let calls = vec![call("1", "read_a"), call("2", "read_b")];

        let started = Instant::now();
        let results = execute_turn_tool_calls(&calls, &registry, "test", 4).await;
        let elapsed = started.elapsed();

        assert!(
            elapsed < Duration::from_millis(350),
            "read-only calls should run concurrently, took {elapsed:?}"
        );
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|(_, is_error)| !is_error));
    }

    #[tokio::test]
    async fn write_tool_is_barrier() {
        let log: CallLog = Arc::new(Mutex::new(Vec::new()));
        let registry = ToolRegistry::new()
            .register(DelayTool::new("read1", true, 100).with_log(&log))
            .register(DelayTool::new("write", false, 100).with_log(&log))
            .register(DelayTool::new("read2", true, 100).with_log(&log));
        let calls = vec![call("1", "read1"), call("2", "write"), call("3", "read2")];

        let results = execute_turn_tool_calls(&calls, &registry, "test", 4).await;
        assert_eq!(results.len(), 3);

        let (_, read1_end) = entry(&log, "read1");
        let (write_start, write_end) = entry(&log, "write");
        let (read2_start, _) = entry(&log, "read2");

        assert!(write_start >= read1_end, "write must wait for read1");
        assert!(read2_start >= write_end, "read2 must wait for write");
    }

    #[tokio::test]
    async fn tool_results_keep_call_order() {
        let registry = ToolRegistry::new()
            .register(DelayTool::new("a", true, 150))
            .register(DelayTool::new("b", true, 50))
            .register(DelayTool::new("c", true, 100));
        let calls = vec![call("1", "a"), call("2", "b"), call("3", "c")];

        let results = execute_turn_tool_calls(&calls, &registry, "test", 4).await;

        assert_eq!(
            results,
            vec![
                ("a".to_string(), false),
                ("b".to_string(), false),
                ("c".to_string(), false),
            ]
        );
    }

    #[tokio::test]
    async fn max_parallel_tools_one_is_sequential() {
        let cur = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let registry = ToolRegistry::new()
            .register(DelayTool::new("a", true, 50).with_counters(&cur, &peak))
            .register(DelayTool::new("b", true, 50).with_counters(&cur, &peak))
            .register(DelayTool::new("c", true, 50).with_counters(&cur, &peak));
        let calls = vec![call("1", "a"), call("2", "b"), call("3", "c")];

        let results = execute_turn_tool_calls(&calls, &registry, "test", 1).await;

        assert_eq!(results.len(), 3);
        assert_eq!(peak.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn read_only_group_respects_max_parallel() {
        let cur = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let registry = ToolRegistry::new()
            .register(DelayTool::new("a", true, 50).with_counters(&cur, &peak))
            .register(DelayTool::new("b", true, 50).with_counters(&cur, &peak))
            .register(DelayTool::new("c", true, 50).with_counters(&cur, &peak));
        let calls = vec![call("1", "a"), call("2", "b"), call("3", "c")];

        let results = execute_turn_tool_calls(&calls, &registry, "test", 2).await;

        assert_eq!(results.len(), 3);
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn max_parallel_zero_is_floored_to_one() {
        let registry = ToolRegistry::new().register(DelayTool::new("a", true, 10));
        let calls = vec![call("1", "a")];

        let results = execute_turn_tool_calls(&calls, &registry, "test", 0).await;

        assert_eq!(results, vec![("a".to_string(), false)]);
    }

    #[tokio::test]
    async fn unknown_tool_call_is_treated_as_barrier() {
        let registry = ToolRegistry::new();
        let calls = vec![call("1", "missing")];

        let results = execute_turn_tool_calls(&calls, &registry, "test", 4).await;

        assert_eq!(results, vec![("Unknown tool: missing".to_string(), true)]);
    }

    #[tokio::test]
    async fn empty_tool_calls_return_empty_results() {
        let registry = ToolRegistry::new();

        let results = execute_turn_tool_calls(&[], &registry, "test", 4).await;

        assert!(results.is_empty());
    }
}
