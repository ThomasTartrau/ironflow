//! Tool for fetching URLs and returning their content.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use reqwest::Client;
use reqwest::redirect::Policy;
use serde_json::{Value, json};
use url::Url;

use super::tool_trait::{Tool, ToolError, ToolOutput};
use crate::ssrf::{self, AllowedHosts, GuardedResolver};

/// Default fetch timeout (30 seconds).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum response body size (5 MB).
const MAX_BODY_SIZE: usize = 5 * 1024 * 1024;

/// Maximum number of redirects followed for one fetch.
const MAX_REDIRECTS: usize = 5;

/// Fetches a URL and returns its content as text.
///
/// Suitable for retrieving web pages, API responses, or any HTTP resource.
/// HTML content is returned as-is (the model can parse it).
///
/// The URL comes from the model, so it is untrusted: a host that is, or resolves to, a
/// private, loopback, link-local or cloud metadata address is refused, on the first
/// request and on every redirect. Decimal, hex, octal and IPv4-mapped forms of those
/// addresses are refused too. [`allow_host`](Self::allow_host) exempts an internal host
/// the deployment wants the agent to read. Proxy environment variables are ignored: a
/// proxy would resolve the target itself, out of reach of this check.
pub struct WebFetchTool {
    timeout: Duration,
    allowed_hosts: AllowedHosts,
    client: Client,
}

impl WebFetchTool {
    /// Create a new `WebFetchTool` with default settings.
    pub fn new() -> Self {
        Self::with_timeout(DEFAULT_TIMEOUT)
    }

    /// Create with a custom timeout.
    pub fn with_timeout(timeout: Duration) -> Self {
        let allowed_hosts = AllowedHosts::default();
        let client = build_client(timeout, &allowed_hosts);
        Self {
            timeout,
            allowed_hosts,
            client,
        }
    }

    /// Let the agent fetch `host` even when it is, or resolves to, an internal address.
    ///
    /// Pass the host as it appears in the URL (`"docs.internal"`, `"10.0.0.5"`, `"::1"`);
    /// the match ignores case and IPv6 brackets. A redirect from an allowed host to a host
    /// that is not allowed is still checked.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::tools::web_fetch::WebFetchTool;
    ///
    /// let tool = WebFetchTool::new().allow_host("docs.internal");
    /// ```
    ///
    /// # Panics
    ///
    /// Panics if the HTTP client cannot be built (TLS backend initialization failure).
    pub fn allow_host(mut self, host: &str) -> Self {
        self.allowed_hosts.add(host);
        self.client = build_client(self.timeout, &self.allowed_hosts);
        self
    }
}

/// Builds the client: [`GuardedResolver`] checks every name it connects to, including
/// redirect targets, and the redirect policy checks IP literals, which skip the resolver.
fn build_client(timeout: Duration, allowed_hosts: &AllowedHosts) -> Client {
    let redirect_allowed = allowed_hosts.clone();
    let redirect = Policy::custom(move |attempt| {
        if attempt.previous().len() > MAX_REDIRECTS {
            return attempt.error(format!("too many redirects (max {MAX_REDIRECTS})"));
        }
        if redirect_allowed.contains_url_host(attempt.url()) {
            return attempt.follow();
        }
        match ssrf::blocked_literal(attempt.url()) {
            Some(blocked) => attempt.error(blocked),
            None => attempt.follow(),
        }
    });
    Client::builder()
        .timeout(timeout)
        .user_agent("ironflow/1.0")
        .redirect(redirect)
        .no_proxy()
        .dns_resolver(GuardedResolver::new(allowed_hosts.clone()))
        .build()
        .expect("failed to build reqwest client")
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

    fn read_only(&self) -> bool {
        true
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

            let parsed = match Url::parse(url) {
                Ok(parsed) => parsed,
                Err(e) => return Ok(ToolOutput::error(format!("Invalid URL {url}: {e}"))),
            };
            if !self.allowed_hosts.contains_url_host(&parsed)
                && let Err(blocked) = ssrf::check_url(&parsed).await
            {
                return Ok(ToolOutput::error(blocked.to_string()));
            }

            let response = match self.client.get(parsed).send().await {
                Ok(r) => r,
                Err(e) => {
                    // A redirect or a DNS answer refused by the guard surfaces deep
                    // in the source chain.
                    return Ok(ToolOutput::error(match ssrf::find_blocked(&e) {
                        Some(blocked) => blocked.to_string(),
                        None => format!("Request failed: {e}"),
                    }));
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
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::task::JoinHandle;

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

    #[test]
    fn web_fetch_tool_is_read_only() {
        assert!(WebFetchTool::new().read_only());
    }

    /// Answers each incoming connection on 127.0.0.1 with the next of `responses`:
    /// a real internal service the guard must keep out of reach.
    async fn serve(responses: Vec<String>) -> (u16, JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            for response in responses {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await.unwrap();
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
            }
        });
        (port, server)
    }

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    fn redirect(location: &str) -> String {
        format!(
            "HTTP/1.1 302 Found\r\nConnection: close\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
        )
    }

    async fn fetch(tool: &WebFetchTool, url: &str) -> ToolOutput {
        tool.execute(json!({ "url": url })).await.unwrap()
    }

    fn assert_blocked(output: &ToolOutput, url: &str) {
        assert!(output.is_error, "{url} should fail, got {}", output.content);
        assert!(
            output.content.contains("blocked IP address"),
            "{url}: {}",
            output.content
        );
        assert!(!output.content.contains("internal secret"), "{url}");
    }

    #[tokio::test]
    async fn web_fetch_blocks_internal_ip_literals() {
        let (port, server) = serve(vec![ok("internal secret")]).await;
        let tool = WebFetchTool::with_timeout(Duration::from_secs(2));
        for url in [
            format!("http://127.0.0.1:{port}/"),
            format!("http://[::1]:{port}/"),
            format!("http://[::ffff:127.0.0.1]:{port}/"),
            "http://169.254.169.254/latest/meta-data/".to_string(),
            "http://10.0.0.1/".to_string(),
            "http://172.16.0.1/".to_string(),
            "http://192.168.1.1/".to_string(),
            "http://[fe80::1]/".to_string(),
            "http://[fd00:ec2::254]/".to_string(),
        ] {
            assert_blocked(&fetch(&tool, &url).await, &url);
        }
        server.abort();
    }

    #[tokio::test]
    async fn web_fetch_blocks_decimal_hex_and_octal_loopback() {
        let (port, server) = serve(vec![ok("internal secret")]).await;
        let tool = WebFetchTool::with_timeout(Duration::from_secs(2));
        for url in [
            format!("http://2130706433:{port}/"),
            format!("http://0x7f000001:{port}/"),
            format!("http://0177.0.0.1:{port}/"),
        ] {
            assert_blocked(&fetch(&tool, &url).await, &url);
        }
        server.abort();
    }

    #[tokio::test]
    async fn web_fetch_blocks_name_resolving_to_loopback() {
        let (port, server) = serve(vec![ok("internal secret")]).await;
        let url = format!("http://localhost:{port}/secret");
        let output = fetch(&WebFetchTool::new(), &url).await;
        server.abort();
        assert_blocked(&output, &url);
        assert!(output.content.contains("URL host localhost resolves to"));
    }

    #[tokio::test]
    async fn web_fetch_allowed_host_returns_content() {
        let (port, server) = serve(vec![ok("internal docs")]).await;
        let tool = WebFetchTool::new().allow_host("localhost");
        let output = fetch(&tool, &format!("http://localhost:{port}/docs")).await;
        server.await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert_eq!(output.content, "internal docs");
    }

    #[tokio::test]
    async fn web_fetch_redirect_to_internal_ip_literal_blocked() {
        let (target, target_server) = serve(vec![ok("internal secret")]).await;
        let (port, server) =
            serve(vec![redirect(&format!("http://127.0.0.1:{target}/secret"))]).await;
        let tool = WebFetchTool::new().allow_host("localhost");
        let url = format!("http://localhost:{port}/");
        let output = fetch(&tool, &url).await;
        server.await.unwrap();
        target_server.abort();
        assert_blocked(&output, &url);
    }

    #[tokio::test]
    async fn web_fetch_redirect_to_name_resolving_internally_blocked() {
        let (target, target_server) = serve(vec![ok("internal secret")]).await;
        let (port, server) =
            serve(vec![redirect(&format!("http://localhost:{target}/secret"))]).await;
        let tool = WebFetchTool::new().allow_host("127.0.0.1");
        let url = format!("http://127.0.0.1:{port}/");
        let output = fetch(&tool, &url).await;
        server.await.unwrap();
        target_server.abort();
        assert_blocked(&output, &url);
    }

    #[tokio::test]
    async fn web_fetch_follows_redirect_to_allowed_host() {
        let (target, target_server) = serve(vec![ok("moved here")]).await;
        let (port, server) = serve(vec![redirect(&format!("http://localhost:{target}/new"))]).await;
        let tool = WebFetchTool::new().allow_host("localhost");
        let output = fetch(&tool, &format!("http://localhost:{port}/old")).await;
        server.await.unwrap();
        target_server.await.unwrap();
        assert!(!output.is_error, "{}", output.content);
        assert_eq!(output.content, "moved here");
    }

    #[tokio::test]
    async fn web_fetch_stops_after_five_redirects() {
        let (port, server) = serve(vec![redirect("/loop"); 6]).await;
        let tool = WebFetchTool::new().allow_host("localhost");
        let output = fetch(&tool, &format!("http://localhost:{port}/loop")).await;
        server.await.unwrap();
        assert!(output.is_error);
        assert!(
            output.content.contains("error following redirect"),
            "{}",
            output.content
        );
    }

    #[tokio::test]
    async fn web_fetch_malformed_url() {
        let tool = WebFetchTool::new();
        for url in ["http://", "http://[::1/"] {
            let output = fetch(&tool, url).await;
            assert!(output.is_error, "{url}");
            assert!(
                output.content.starts_with("Invalid URL"),
                "{}",
                output.content
            );
        }
    }
}
