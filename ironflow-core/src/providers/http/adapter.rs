//! Core adapter trait and generic provider wrapper for HTTP-based LLM APIs.

use std::time::{Duration, Instant};

use reqwest::Client;
use serde_json::Value;
use tracing::info;

use crate::error::AgentError;
use crate::provider::{AgentConfig, AgentOutput, AgentProvider, DebugMessage, InvokeFuture};
use crate::providers::http::sse::{SseDelta, collect_sse_stream};

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
    /// Input/prompt tokens consumed.
    pub input_tokens: Option<u64>,
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

    /// Compute cost in USD from token counts. Returns `None` if unknown.
    fn compute_cost(&self, model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64>;

    /// Resolve model alias (e.g. "sonnet") to a provider-specific model ID.
    fn resolve_model(&self, model: &str) -> String;
}

/// Default timeout for HTTP provider requests.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);

/// Generic HTTP provider that wraps any [`HttpAgentAdapter`].
///
/// Implements [`AgentProvider`] by delegating request construction and response
/// parsing to the adapter while handling the HTTP transport, timeout, and
/// single-turn execution loop.
pub struct HttpAgentProvider<A: HttpAgentAdapter> {
    adapter: A,
    client: Client,
    timeout: Duration,
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
        }
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

        let response = tokio::time::timeout(self.timeout, req.send())
            .await
            .map_err(|_| AgentError::Timeout {
                limit: self.timeout,
            })?
            .map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("HTTP request failed: {e}"),
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
            let body: Value = response.json().await.map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to parse response JSON: {e}"),
            })?;
            self.adapter.parse_response(&body, config)
        }
    }
}

impl<A: HttpAgentAdapter> AgentProvider for HttpAgentProvider<A> {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
            let start = Instant::now();

            let request_body = self.adapter.build_request(config)?;
            let turn_result = self.execute_turn(&request_body, config).await?;

            let duration_ms = start.elapsed().as_millis() as u64;
            let input_tokens = turn_result.usage.input_tokens.unwrap_or(0);
            let output_tokens = turn_result.usage.output_tokens.unwrap_or(0);
            let model_name = turn_result.model.clone();

            let cost = model_name
                .as_deref()
                .and_then(|m| self.adapter.compute_cost(m, input_tokens, output_tokens));

            let debug_messages = if config.verbose {
                Some(vec![DebugMessage {
                    text: turn_result.text.clone(),
                    thinking: None,
                    thinking_redacted: false,
                    tool_calls: Vec::new(),
                    tool_results: Vec::new(),
                    stop_reason: if turn_result.is_final {
                        Some("end_turn".to_string())
                    } else {
                        Some("tool_use".to_string())
                    },
                    input_tokens: turn_result.usage.input_tokens,
                    output_tokens: turn_result.usage.output_tokens,
                }])
            } else {
                None
            };

            let value = if let Some(structured) = turn_result.structured_value {
                structured
            } else {
                turn_result
                    .text
                    .map(Value::String)
                    .unwrap_or(Value::String(String::new()))
            };

            info!(
                provider = self.adapter.provider_name(),
                duration_ms,
                input_tokens,
                output_tokens,
                "invocation complete"
            );

            Ok(AgentOutput {
                value,
                session_id: None,
                cost_usd: cost,
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
                model: model_name,
                duration_ms,
                debug_messages,
            })
        })
    }
}
