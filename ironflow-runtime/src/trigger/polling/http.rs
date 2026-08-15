//! HTTP polling probe.
//!
//! [`HttpProbe`] polls an HTTP endpoint at each interval and triggers when
//! the response body changes. The body is hashed with SHA-256 for
//! deduplication.
//!
//! # Feature flag
//!
//! This module is only available when the `trigger-polling-http` feature is
//! enabled.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_runtime::trigger::polling::http::{HttpProbe, HttpProbeConfig};
//!
//! let probe = HttpProbe::new(HttpProbeConfig {
//!     url: "https://api.example.com/status".to_string(),
//!     method: "GET".to_string(),
//!     headers: vec![("Authorization".to_string(), "Bearer tok".to_string())],
//!     expected_status: 200,
//! });
//! ```

use reqwest::Client;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::{PollingProbe, ProbeError, ProbeFuture, ProbeResult};

/// Configuration for an [`HttpProbe`].
///
/// # Examples
///
/// ```
/// use ironflow_runtime::trigger::polling::http::HttpProbeConfig;
///
/// let config = HttpProbeConfig {
///     url: "https://example.com/data".to_string(),
///     method: "GET".to_string(),
///     headers: vec![],
///     expected_status: 200,
/// };
/// assert_eq!(config.url, "https://example.com/data");
/// ```
#[derive(Debug, Clone)]
pub struct HttpProbeConfig {
    /// The URL to poll.
    pub url: String,
    /// HTTP method (GET, POST, etc.).
    pub method: String,
    /// Headers to include in the request.
    pub headers: Vec<(String, String)>,
    /// Expected HTTP status code. A different status is treated as an error.
    pub expected_status: u16,
}

/// An HTTP polling probe that triggers when the response body changes.
///
/// # Examples
///
/// ```no_run
/// use ironflow_runtime::trigger::polling::http::{HttpProbe, HttpProbeConfig};
/// use ironflow_runtime::trigger::polling::PollingProbe;
///
/// let probe = HttpProbe::new(HttpProbeConfig {
///     url: "https://api.example.com/feed".to_string(),
///     method: "GET".to_string(),
///     headers: vec![],
///     expected_status: 200,
/// });
/// assert_eq!(probe.name(), "http");
/// ```
pub struct HttpProbe {
    config: HttpProbeConfig,
    client: Client,
}

impl HttpProbe {
    /// Create a new HTTP probe with the given configuration.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_runtime::trigger::polling::http::{HttpProbe, HttpProbeConfig};
    ///
    /// let probe = HttpProbe::new(HttpProbeConfig {
    ///     url: "https://example.com".to_string(),
    ///     method: "GET".to_string(),
    ///     headers: vec![],
    ///     expected_status: 200,
    /// });
    /// ```
    pub fn new(config: HttpProbeConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

impl PollingProbe for HttpProbe {
    fn name(&self) -> &str {
        "http"
    }

    fn poll(&self) -> ProbeFuture<'_> {
        Box::pin(async {
            let method = self
                .config
                .method
                .parse()
                .map_err(|e| ProbeError::Failed(format!("invalid HTTP method: {e}")))?;

            let mut request = self.client.request(method, &self.config.url);
            for (key, value) in &self.config.headers {
                request = request.header(key.as_str(), value.as_str());
            }

            let response = request
                .send()
                .await
                .map_err(|e| ProbeError::Failed(format!("HTTP request failed: {e}")))?;

            let status = response.status().as_u16();
            if status != self.config.expected_status {
                return Err(ProbeError::Failed(format!(
                    "unexpected status: {status}, expected: {}",
                    self.config.expected_status
                )));
            }

            let body = response
                .bytes()
                .await
                .map_err(|e| ProbeError::Failed(format!("failed to read body: {e}")))?;

            let content_hash = hex::encode(Sha256::digest(&body));

            let data = match serde_json::from_slice(&body) {
                Ok(v) => v,
                Err(_) => {
                    let body_str = String::from_utf8_lossy(&body);
                    json!({ "body": body_str })
                }
            };

            Ok(Some(ProbeResult::with_hash(data, content_hash)))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_probe_name() {
        let probe = HttpProbe::new(HttpProbeConfig {
            url: "http://localhost".to_string(),
            method: "GET".to_string(),
            headers: vec![],
            expected_status: 200,
        });
        assert_eq!(probe.name(), "http");
    }

    #[test]
    fn http_probe_config_clone() {
        let config = HttpProbeConfig {
            url: "http://localhost".to_string(),
            method: "POST".to_string(),
            headers: vec![("X-Key".to_string(), "val".to_string())],
            expected_status: 201,
        };
        let cloned = config.clone();
        assert_eq!(cloned.url, config.url);
        assert_eq!(cloned.method, config.method);
        assert_eq!(cloned.headers.len(), 1);
        assert_eq!(cloned.expected_status, 201);
    }
}
