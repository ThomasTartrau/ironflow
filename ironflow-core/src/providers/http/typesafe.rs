//! TypeSafe AI (Jev / System One) [`DecisionProvider`] over HTTP.
//!
//! Speaks the System One evaluation API: `POST /v1/systemone` with a Bearer
//! token, a JSON body of `{ state, model, questions }`, and a JSON response of
//! `{ model, answers, usage }`. See <https://docs.typesafe.ai/api>.
//!
//! The same wire contract is served by OpenRouter's Decisions endpoint, so the
//! provider reaches Jev either directly (`new`) or through OpenRouter
//! (`openrouter`) with only the base URL and API key changing. See
//! [`TypeSafeProvider::openrouter`].
//!
//! Enabled by the `provider-typesafe` feature.

use std::time::Duration;

use reqwest::header::RETRY_AFTER;
use reqwest::{Client, StatusCode};

use crate::decision::{DecideFuture, DecisionOutput, DecisionProvider, DecisionRequest};
use crate::error::AgentError;

/// Default System One evaluation endpoint.
pub const DEFAULT_ENDPOINT: &str = "https://api.typesafe.ai/v1/systemone";

/// Default model route for early access.
pub const DEFAULT_MODEL: &str = "jev-latest";

/// OpenRouter's Decisions endpoint, which serves the same System One wire
/// contract as the native TypeSafe API.
///
/// This is an `alpha` route and may move; override it with
/// [`TypeSafeProvider::with_endpoint`] if OpenRouter relocates it.
pub const OPENROUTER_ENDPOINT: &str = "https://openrouter.ai/api/alpha/decisions";

/// Model slug to request when routing through OpenRouter.
///
/// OpenRouter requires the concrete, versioned TypeSafe slug: the native
/// [`DEFAULT_MODEL`] alias (`jev-latest`) and its namespaced form
/// `typesafe/jev-latest` both return `400 "Model does not exist"`. Set it on
/// the request via `DecisionConfig::model(OPENROUTER_MODEL)`.
pub const OPENROUTER_MODEL: &str = "typesafe/jev-1.13";

const PROVIDER_NAME: &str = "typesafe";

/// A [`DecisionProvider`] backed by the TypeSafe AI System One HTTP API.
///
/// # Examples
///
/// ```
/// use ironflow_core::providers::http::TypeSafeProvider;
///
/// let provider = TypeSafeProvider::new("sk-test");
/// assert_eq!(provider.endpoint(), "https://api.typesafe.ai/v1/systemone");
/// ```
pub struct TypeSafeProvider {
    client: Client,
    api_key: String,
    endpoint: String,
}

impl TypeSafeProvider {
    /// Create a provider with the given API key and default endpoint.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::TypeSafeProvider;
    ///
    /// let provider = TypeSafeProvider::new("sk-test");
    /// ```
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            api_key: api_key.into(),
            endpoint: DEFAULT_ENDPOINT.to_string(),
        }
    }

    /// Create a provider that routes to Jev through OpenRouter's Decisions
    /// endpoint ([`OPENROUTER_ENDPOINT`]) with an OpenRouter API key.
    ///
    /// The wire contract is identical to the native TypeSafe API; only the base
    /// URL and key differ. Remember to select the OpenRouter model slug
    /// ([`OPENROUTER_MODEL`]) on the request, since OpenRouter rejects the
    /// `jev-latest` alias and requires the concrete versioned slug.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::typesafe::OPENROUTER_ENDPOINT;
    /// use ironflow_core::providers::http::TypeSafeProvider;
    ///
    /// let provider = TypeSafeProvider::openrouter("sk-or-test");
    /// assert_eq!(provider.endpoint(), OPENROUTER_ENDPOINT);
    /// ```
    pub fn openrouter(api_key: impl Into<String>) -> Self {
        Self::new(api_key).with_endpoint(OPENROUTER_ENDPOINT)
    }

    /// Override the endpoint (e.g. a local test server or a proxy).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::TypeSafeProvider;
    ///
    /// let provider = TypeSafeProvider::new("sk-test")
    ///     .with_endpoint("http://127.0.0.1:8080/v1/systemone");
    /// assert_eq!(provider.endpoint(), "http://127.0.0.1:8080/v1/systemone");
    /// ```
    pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.endpoint = endpoint.into();
        self
    }

    /// Supply a preconfigured [`reqwest::Client`] (custom timeout, proxy, TLS).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::TypeSafeProvider;
    /// use reqwest::Client;
    ///
    /// let provider = TypeSafeProvider::new("sk-test").with_client(Client::new());
    /// ```
    pub fn with_client(mut self, client: Client) -> Self {
        self.client = client;
        self
    }

    /// The endpoint this provider posts to.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

impl DecisionProvider for TypeSafeProvider {
    fn decide<'a>(&'a self, request: &'a DecisionRequest) -> DecideFuture<'a> {
        Box::pin(async move {
            let response = self
                .client
                .post(&self.endpoint)
                .bearer_auth(&self.api_key)
                .json(request)
                .send()
                .await
                .map_err(|e| AgentError::HttpProvider {
                    provider: PROVIDER_NAME.to_string(),
                    status_code: 0,
                    message: e.to_string(),
                })?;

            let status = response.status();

            if status == StatusCode::TOO_MANY_REQUESTS {
                let retry_after_secs = response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok());
                return Err(AgentError::RateLimited {
                    provider: PROVIDER_NAME.to_string(),
                    retry_after_secs,
                });
            }

            if !status.is_success() {
                let code = status.as_u16();
                let message = response.text().await.unwrap_or_default();
                return Err(AgentError::HttpProvider {
                    provider: PROVIDER_NAME.to_string(),
                    status_code: code,
                    message,
                });
            }

            let body = response
                .text()
                .await
                .map_err(|e| AgentError::HttpProvider {
                    provider: PROVIDER_NAME.to_string(),
                    status_code: status.as_u16(),
                    message: format!("failed to read response body: {e}"),
                })?;

            serde_json::from_str::<DecisionOutput>(&body).map_err(|e| AgentError::HttpProvider {
                provider: PROVIDER_NAME.to_string(),
                status_code: status.as_u16(),
                message: format!("failed to parse decision response: {e}"),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_uses_default_endpoint() {
        let provider = TypeSafeProvider::new("sk");
        assert_eq!(provider.endpoint(), DEFAULT_ENDPOINT);
    }

    #[test]
    fn with_endpoint_overrides() {
        let provider = TypeSafeProvider::new("sk").with_endpoint("http://localhost/x");
        assert_eq!(provider.endpoint(), "http://localhost/x");
    }

    #[test]
    fn openrouter_uses_alpha_decisions_endpoint() {
        let provider = TypeSafeProvider::openrouter("sk-or");
        assert_eq!(provider.endpoint(), OPENROUTER_ENDPOINT);
        assert_eq!(OPENROUTER_MODEL, "typesafe/jev-1.13");
    }
}
