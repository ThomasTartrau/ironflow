//! HTTP step executor.

use std::sync::Arc;
use std::time::{Duration, Instant};

use rust_decimal::Decimal;
use serde_json::json;
use tracing::info;

use ironflow_core::operations::http::Http;
use ironflow_core::provider::AgentProvider;

use crate::config::HttpConfig;
use crate::error::EngineError;

use super::{StepExecutor, StepOutput};

/// Executor for HTTP steps.
///
/// Sends an HTTP request and captures the response status, headers, and body.
pub struct HttpExecutor<'a> {
    config: &'a HttpConfig,
}

impl<'a> HttpExecutor<'a> {
    /// Create a new HTTP executor from a config reference.
    pub fn new(config: &'a HttpConfig) -> Self {
        Self { config }
    }
}

impl StepExecutor for HttpExecutor<'_> {
    async fn execute(&self, _provider: &Arc<dyn AgentProvider>) -> Result<StepOutput, EngineError> {
        let start = Instant::now();

        let mut http = match self.config.method.to_uppercase().as_str() {
            "GET" => Http::get(&self.config.url),
            "POST" => Http::post(&self.config.url),
            "PUT" => Http::put(&self.config.url),
            "PATCH" => Http::patch(&self.config.url),
            "DELETE" => Http::delete(&self.config.url),
            other => {
                return Err(EngineError::StepConfig(format!(
                    "unsupported HTTP method: {other}"
                )));
            }
        };

        for (name, value) in &self.config.headers {
            http = http.header(name, value);
        }
        if let Some(ref body) = self.config.body {
            http = http.json(body.clone());
        }
        if let Some(secs) = self.config.timeout_secs {
            http = http.timeout(Duration::from_secs(secs));
        }

        let output = http.run().await?;
        let duration_ms = start.elapsed().as_millis() as u64;

        info!(
            step_kind = "http",
            method = %self.config.method,
            url = %self.config.url,
            status = output.status(),
            duration_ms,
            "http step completed"
        );

        Ok(StepOutput {
            output: json!({
                "status": output.status(),
                "headers": output.headers(),
                "body": output.body(),
            }),
            duration_ms,
            cost_usd: Decimal::ZERO,
            input_tokens: None,
            output_tokens: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironflow_core::providers::claude::ClaudeCodeProvider;
    use ironflow_core::providers::record_replay::RecordReplayProvider;

    fn create_test_provider() -> Arc<dyn AgentProvider> {
        let inner = ClaudeCodeProvider::new();
        Arc::new(RecordReplayProvider::replay(
            inner,
            "/tmp/ironflow-fixtures",
        ))
    }

    #[tokio::test]
    async fn http_get_method() {
        let config = HttpConfig::get("http://httpbin.org/status/200");
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.output.get("status").is_some());
        assert!(output.output.get("headers").is_some());
        assert!(output.output.get("body").is_some());
    }

    #[tokio::test]
    async fn http_post_method() {
        let config = HttpConfig::post("http://httpbin.org/post");
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn http_put_method() {
        let config = HttpConfig::put("http://httpbin.org/put");
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn http_patch_method() {
        let config = HttpConfig::patch("http://httpbin.org/patch");
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn http_delete_method() {
        let config = HttpConfig::delete("http://httpbin.org/delete");
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn http_unsupported_method_returns_error() {
        let mut config = HttpConfig::get("http://httpbin.org/status/200");
        config.method = "INVALID".to_string();
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_err());
        match result {
            Err(EngineError::StepConfig(msg)) => {
                assert!(msg.contains("unsupported HTTP method"));
            }
            _ => panic!("expected StepConfig error"),
        }
    }

    #[tokio::test]
    async fn http_with_custom_headers() {
        let config = HttpConfig::get("http://httpbin.org/headers")
            .header("X-Custom-Header", "test-value")
            .header("Authorization", "Bearer token");
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn http_with_json_body() {
        let config =
            HttpConfig::post("http://httpbin.org/post").json(json!({"key": "value", "number": 42}));
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let result = executor.execute(&provider).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn http_step_output_has_structure() {
        let config = HttpConfig::get("http://httpbin.org/status/200");
        let executor = HttpExecutor::new(&config);
        let provider = create_test_provider();

        let output = executor.execute(&provider).await.unwrap();
        assert!(output.output.get("status").is_some());
        assert!(output.output.get("headers").is_some());
        assert!(output.output.get("body").is_some());
        assert_eq!(output.cost_usd, Decimal::ZERO);
        assert!(output.duration_ms > 0);
    }
}
