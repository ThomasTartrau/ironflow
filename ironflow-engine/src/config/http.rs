//! [`HttpConfig`] — serializable configuration for an HTTP step.

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
}
