//! Tool for web search via a configurable search API.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::Client;
use serde_json::{Value, json};
use url::Url;

use super::tool_trait::{Tool, ToolError, ToolOutput};

/// Default search timeout (15 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// Performs web searches via a configurable search API.
///
/// Supports Brave Search API by default, but can be configured for
/// any search API that accepts a query parameter and returns JSON results.
pub struct WebSearchTool {
    client: Client,
    api_key: String,
    endpoint: String,
    query_param: String,
    api_key_header: String,
}

impl WebSearchTool {
    /// Create a `WebSearchTool` configured for the Brave Search API.
    pub fn brave(api_key: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            api_key: api_key.into(),
            endpoint: "https://api.search.brave.com/res/v1/web/search".to_string(),
            query_param: "q".to_string(),
            api_key_header: "X-Subscription-Token".to_string(),
        }
    }

    /// Create a `WebSearchTool` with custom endpoint configuration.
    pub fn custom(
        api_key: impl Into<String>,
        endpoint: impl Into<String>,
        query_param: impl Into<String>,
        api_key_header: impl Into<String>,
    ) -> Self {
        let client = Client::builder()
            .timeout(DEFAULT_TIMEOUT)
            .build()
            .expect("failed to build reqwest client");
        Self {
            client,
            api_key: api_key.into(),
            endpoint: endpoint.into(),
            query_param: query_param.into(),
            api_key_header: api_key_header.into(),
        }
    }
}

impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns a list of relevant results with titles, URLs, and snippets."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "count": {
                    "type": "integer",
                    "description": "Number of results to return (default: 5, max: 20)"
                }
            },
            "required": ["query"]
        })
    }

    fn execute(
        &self,
        input: Value,
    ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
        Box::pin(async move {
            let query = input
                .get("query")
                .and_then(|v| v.as_str())
                .ok_or_else(|| ToolError::new("missing 'query' parameter"))?;

            let count = input
                .get("count")
                .and_then(|v| v.as_u64())
                .unwrap_or(5)
                .min(20);

            let mut url = Url::parse(&self.endpoint)
                .map_err(|e| ToolError::new(format!("invalid search endpoint URL: {e}")))?;
            url.query_pairs_mut()
                .append_pair(&self.query_param, query)
                .append_pair("count", &count.to_string());

            let response = match self
                .client
                .get(url.as_str())
                .header(&self.api_key_header, &self.api_key)
                .send()
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    return Ok(ToolOutput::error(format!("Search request failed: {e}")));
                }
            };

            let status = response.status().as_u16();
            if status >= 400 {
                let error_body = response.text().await.unwrap_or_default();
                return Ok(ToolOutput::error(format!(
                    "Search API returned HTTP {status}: {error_body}"
                )));
            }

            let body: Value = match response.json().await {
                Ok(v) => v,
                Err(e) => {
                    return Ok(ToolOutput::error(format!(
                        "Failed to parse search response: {e}"
                    )));
                }
            };

            let results = extract_search_results(&body);
            if results.is_empty() {
                return Ok(ToolOutput::success("No results found for the given query."));
            }

            Ok(ToolOutput::success(results))
        })
    }
}

/// Extract search results from a Brave Search API response.
///
/// Falls back to a generic extraction if the Brave-specific structure
/// is not found.
fn extract_search_results(body: &Value) -> String {
    // Brave Search format
    if let Some(results) = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
    {
        return format_results(results);
    }

    // Generic: try "results" array at top level
    if let Some(results) = body.get("results").and_then(|r| r.as_array()) {
        return format_results(results);
    }

    // Fallback: return the raw JSON (truncated)
    let raw = body.to_string();
    if raw.len() > 4000 {
        format!("{}\n... (truncated)", &raw[..raw.floor_char_boundary(4000)])
    } else {
        raw
    }
}

fn format_results(results: &[Value]) -> String {
    results
        .iter()
        .enumerate()
        .map(|(i, result)| {
            let title = result
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("(no title)");
            let url = result
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("(no url)");
            let description = result
                .get("description")
                .or_else(|| result.get("snippet"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            format!("{}. {}\n   {}\n   {}", i + 1, title, url, description)
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn extract_brave_format() {
        let body = json!({
            "web": {
                "results": [
                    {"title": "Rust Lang", "url": "https://rust-lang.org", "description": "A language"}
                ]
            }
        });
        let result = extract_search_results(&body);
        assert!(result.contains("Rust Lang"));
        assert!(result.contains("https://rust-lang.org"));
    }

    #[test]
    fn extract_generic_format() {
        let body = json!({
            "results": [
                {"title": "Test", "url": "https://test.com", "snippet": "A test page"}
            ]
        });
        let result = extract_search_results(&body);
        assert!(result.contains("Test"));
        assert!(result.contains("A test page"));
    }

    #[test]
    fn extract_empty_results() {
        let body = json!({"web": {"results": []}});
        let result = extract_search_results(&body);
        assert!(result.is_empty());
    }

    #[test]
    fn parameters_schema_has_required_query() {
        let tool = WebSearchTool::brave("fake-key");
        let schema = tool.parameters_schema();
        assert_eq!(schema["required"][0], "query");
    }

    #[tokio::test]
    async fn web_search_missing_query() {
        let tool = WebSearchTool::brave("fake-key");
        let result = tool.execute(json!({})).await;
        assert!(result.is_err());
    }
}
