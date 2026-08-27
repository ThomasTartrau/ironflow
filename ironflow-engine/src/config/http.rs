//! [`HttpConfig`] — serializable configuration for an HTTP step.

use ironflow_core::retry::RetryPolicy;
use ironflow_core::trace_context::WorkflowTraceContext;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Serializable configuration for an HTTP step.
///
/// # Examples
///
/// ```
/// use ironflow_engine::config::HttpConfig;
///
/// let config = HttpConfig::get("https://api.example.com/health");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpConfig {
    /// HTTP method (GET, POST, PUT, PATCH, DELETE).
    pub method: String,
    /// Request URL.
    pub url: String,
    /// Request headers.
    pub headers: Vec<(String, String)>,
    /// Request body as JSON.
    pub body: Option<Value>,
    /// Timeout in seconds (default: 30).
    pub timeout_secs: Option<u64>,
    /// When `true`, a failure of this step does not fail the run.
    #[serde(default)]
    pub allow_failure: bool,
    /// Optional step-level retry policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry: Option<RetryPolicy>,
    /// Optional W3C trace context for distributed tracing propagation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_context: Option<WorkflowTraceContext>,
}

impl HttpConfig {
    /// Create a GET request config.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HttpConfig;
    ///
    /// let config = HttpConfig::get("https://example.com");
    /// assert_eq!(config.method, "GET");
    /// ```
    pub fn get(url: &str) -> Self {
        Self::new("GET", url)
    }

    /// Create a POST request config.
    pub fn post(url: &str) -> Self {
        Self::new("POST", url)
    }

    /// Create a PUT request config.
    pub fn put(url: &str) -> Self {
        Self::new("PUT", url)
    }

    /// Create a PATCH request config.
    pub fn patch(url: &str) -> Self {
        Self::new("PATCH", url)
    }

    /// Create a DELETE request config.
    pub fn delete(url: &str) -> Self {
        Self::new("DELETE", url)
    }

    fn new(method: &str, url: &str) -> Self {
        Self {
            method: method.to_string(),
            url: url.to_string(),
            headers: Vec::new(),
            body: None,
            timeout_secs: None,
            allow_failure: false,
            retry: None,
            trace_context: None,
        }
    }

    /// Add a request header.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Set the request body as JSON.
    pub fn json(mut self, body: Value) -> Self {
        self.body = Some(body);
        self
    }

    /// Set the timeout in seconds.
    pub fn timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Mark this step as allowed to fail without stopping the run.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_engine::config::HttpConfig;
    ///
    /// let config = HttpConfig::get("https://example.com").allow_failure();
    /// assert!(config.allow_failure);
    /// ```
    pub fn allow_failure(mut self) -> Self {
        self.allow_failure = true;
        self
    }

    /// Set a step-level retry policy.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::retry::RetryPolicy;
    /// use ironflow_engine::config::HttpConfig;
    ///
    /// let config = HttpConfig::get("https://api.example.com")
    ///     .retry_policy(RetryPolicy::new(5));
    /// assert!(config.retry.is_some());
    /// ```
    pub fn retry_policy(mut self, policy: RetryPolicy) -> Self {
        self.retry = Some(policy);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn methods() {
        assert_eq!(HttpConfig::get("http://x").method, "GET");
        assert_eq!(HttpConfig::post("http://x").method, "POST");
        assert_eq!(HttpConfig::put("http://x").method, "PUT");
        assert_eq!(HttpConfig::patch("http://x").method, "PATCH");
        assert_eq!(HttpConfig::delete("http://x").method, "DELETE");
    }

    #[test]
    fn builder() {
        let config = HttpConfig::post("http://api.example.com")
            .header("Authorization", "Bearer token")
            .json(json!({"key": "value"}))
            .timeout_secs(10);

        assert_eq!(config.headers.len(), 1);
        assert!(config.body.is_some());
        assert_eq!(config.timeout_secs, Some(10));
    }

    #[test]
    fn a_config_predating_retry_still_deserializes() {
        let config: HttpConfig = serde_json::from_str(
            r#"{"url":"http://x","method":"GET","headers":[],"body":null,"timeout_secs":null}"#,
        )
        .expect("deserialize");
        assert!(config.retry.is_none());
    }

    #[test]
    fn retry_policy_roundtrip() {
        use ironflow_core::retry::RetryPolicy;

        let config = HttpConfig::get("http://api").retry_policy(RetryPolicy::new(3));
        let json = serde_json::to_string(&config).expect("serialize");
        let back: HttpConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.retry.as_ref().unwrap().max_retries(), 3);
    }
}
