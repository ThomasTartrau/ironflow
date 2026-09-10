//! Builder pattern for [`IronflowClient`] with retry and rate-limit configuration.

use std::time::Duration;

use bytes::Bytes;

use crate::client::{ClientConfig, IronflowClient};
use crate::rate_limit::RateLimiter;
use crate::retry::RetryConfig;

/// Result of downloading an artifact.
///
/// # Examples
///
/// ```
/// use ironflow_sdk::client::ArtifactDownload;
/// use bytes::Bytes;
///
/// let download = ArtifactDownload {
///     bytes: Bytes::from_static(b"hello"),
///     content_type: "text/plain".to_string(),
///     sha256: "abc123".to_string(),
/// };
/// assert_eq!(download.bytes.len(), 5);
/// ```
#[derive(Debug, Clone)]
pub struct ArtifactDownload {
    /// Raw artifact bytes.
    pub bytes: Bytes,
    /// MIME type recorded at upload time.
    pub content_type: String,
    /// SHA-256 digest of the artifact content (hex-encoded).
    pub sha256: String,
}

/// Builder for [`IronflowClient`] with retry and rate-limit configuration.
///
/// # Examples
///
/// ```
/// use ironflow_sdk::ClientBuilder;
///
/// let client = ClientBuilder::new("https://ironflow.example.com", "my-api-key")
///     .with_max_retries(5)
///     .with_rate_limit(false)
///     .build();
/// ```
pub struct ClientBuilder {
    config: ClientConfig,
    retry: RetryConfig,
    rate_limit_enabled: bool,
}

impl ClientBuilder {
    /// Create a builder with defaults (3 retries, rate-limit enabled).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_sdk::ClientBuilder;
    ///
    /// let builder = ClientBuilder::new("https://ironflow.example.com", "key");
    /// ```
    pub fn new(base_url: &str, api_key: &str) -> Self {
        Self {
            config: ClientConfig {
                base_url: base_url.trim_end_matches('/').to_string(),
                api_key: api_key.to_string(),
                timeout: Duration::from_secs(30),
            },
            retry: RetryConfig::default(),
            rate_limit_enabled: true,
        }
    }

    /// Set the maximum number of retry attempts.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_sdk::ClientBuilder;
    ///
    /// let client = ClientBuilder::new("https://ironflow.example.com", "key")
    ///     .with_max_retries(5)
    ///     .build();
    /// ```
    pub fn with_max_retries(mut self, n: u32) -> Self {
        self.retry.max_retries = n;
        self
    }

    /// Set the HTTP request timeout.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_sdk::ClientBuilder;
    /// use std::time::Duration;
    ///
    /// let client = ClientBuilder::new("https://ironflow.example.com", "key")
    ///     .with_timeout(Duration::from_secs(60))
    ///     .build();
    /// ```
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.config.timeout = timeout;
        self
    }

    /// Enable or disable client-side rate limiting.
    ///
    /// When enabled (the default), the client waits for the `Retry-After`
    /// window to expire before sending the next request after a 429.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_sdk::ClientBuilder;
    ///
    /// let client = ClientBuilder::new("https://ironflow.example.com", "key")
    ///     .with_rate_limit(false)
    ///     .build();
    /// ```
    pub fn with_rate_limit(mut self, enabled: bool) -> Self {
        self.rate_limit_enabled = enabled;
        self
    }

    /// Build the [`IronflowClient`].
    ///
    /// # Panics
    ///
    /// Panics if the API key contains characters invalid for HTTP headers.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_sdk::ClientBuilder;
    ///
    /// let client = ClientBuilder::new("https://ironflow.example.com", "key").build();
    /// ```
    pub fn build(self) -> IronflowClient {
        let rate_limiter = if self.rate_limit_enabled {
            RateLimiter::new()
        } else {
            RateLimiter::disabled()
        };
        IronflowClient::from_config_with_retry(self.config, self.retry, rate_limiter)
    }
}
