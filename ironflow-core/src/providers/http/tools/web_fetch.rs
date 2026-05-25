//! Tool for fetching URLs and returning their content.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};

use super::tool_trait::{Tool, ToolError, ToolOutput};

/// Default fetch timeout (30 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum response body size (5 MB).
const MAX_BODY_SIZE: usize = 5 * 1024 * 1024;

/// Fetches a URL and returns its content as text.
///
/// Suitable for retrieving web pages, API responses, or any HTTP resource.
/// HTML content is returned as-is (the model can parse it).
pub struct WebFetchTool {
    client: Client,
}

impl WebFetchTool {
    /// Create a new `WebFetchTool` with default settings.
    pub fn new() -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .user_agent("ironflow/1.0")
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("failed to build reqwest client");
        Self { client }
    }

    /// Create with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        let client = Client::builder()
            .timeout(timeout)
            .user_agent("ironflow/1.0")
            .redirect(reqwest::redirect::Policy::limited(5))
            .build()
            .expect("failed to build reqwest client");
        Self { client }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "Fetch a URL and return its content as text. Supports HTTP and HTTPS."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "The URL to fetch (must start with http:// or https://)"
                }
            },
            "required": ["url"]
        })
    }

    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let url = input
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::new("missing 'url' parameter"))?;

            if !url.starts_with("http://") && !url.starts_with("https://") {
                return Ok(ToolOutput::error("URL must start with http:// or https://"));
            }

            let response = match self.client.get(url).send().await {
                Ok(r) => r,
                Err(e) => {
                    return Ok(ToolOutput::error(format!("Request failed: {e}")));
                }
            };

            let status = response.status().as_u16();
            if status >= 400 {
                return Ok(ToolOutput::error(format!("HTTP {status} for {url}")));
            }

            let content_length = response.content_length().unwrap_or(0);
            if content_length > MAX_BODY_SIZE as u64 {
                return Ok(ToolOutput::error(format!(
                    "Response too large: {} bytes (max {})",
                    content_length, MAX_BODY_SIZE
                )));
            }

            let body = match response.text().await {
                Ok(b) => b,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to read response body: {e}"
                    )));
                }
            };

            if body.len() > MAX_BODY_SIZE {
                let truncated = &body[..body.floor_char_boundary(MAX_BODY_SIZE)];
                Ok(ToolOutput::success(format!(
                    "{truncated}\n... (truncated at {MAX_BODY_SIZE} bytes)"
                )))
            } else {
                Ok(ToolOutput::success(body))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn web_fetch_invalid_url_scheme() {
        let tool = WebFetchTool::new();
        let result = tool
            .execute(json!({"url": "ftp://example.com"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("must start with http"));
    }

    #[tokio::test]
    async fn web_fetch_missing_url() {
        let tool = WebFetchTool::new();
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn web_fetch_nonexistent_host() {
        let tool = WebFetchTool::with_timeout(Duration::from_secs(2));
        let result = tool
            .execute(json!({"url": "http://this-host-does-not-exist-ironflow-test.invalid/"}))
            .await
            .expect("should succeed");
        assert!(result.is_error);
        assert!(result.content.contains("Request failed"));
    }
}
