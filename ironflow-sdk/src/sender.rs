//! Retry-aware request sending for [`IronflowClient`].

use ironflow_types::ApiResponse;
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::from_slice;

use crate::client::IronflowClient;
use crate::error::Error;
use crate::retry::{backoff_delay, is_retryable_error, is_retryable_status, parse_retry_after};

impl IronflowClient {
    /// Send a request with automatic retry on transient failures.
    ///
    /// Retries on 429, 502, 503, 504 and connection/timeout errors.
    /// Respects the `Retry-After` header on 429 responses.
    ///
    /// Only use this for idempotent requests (GET, PUT, DELETE).
    /// For non-idempotent requests, use [`send_once`](Self::send_once).
    ///
    /// # Errors
    ///
    /// Returns [`Error::Exhausted`] when all retry attempts are used up
    /// (for both HTTP status and network errors).
    /// Returns the underlying [`Error::Http`] or [`Error::Api`] on
    /// non-retryable failures.
    pub(crate) async fn send_with_retry(&self, request: RequestBuilder) -> Result<Response, Error> {
        self.rate_limiter.wait().await;

        let max = self.retry_config.max_retries;

        for attempt in 0..=max {
            let req = request.try_clone().ok_or_else(|| {
                Error::Deserialize(
                    "request body is not cloneable (streaming bodies cannot be retried)"
                        .to_string(),
                )
            })?;

            match req.send().await {
                Ok(response) => {
                    let status = response.status();

                    let retry_after = if status == StatusCode::TOO_MANY_REQUESTS {
                        let ra = parse_retry_after(&response);
                        if let Some(duration) = ra {
                            self.rate_limiter.record(duration).await;
                        }
                        ra
                    } else {
                        None
                    };

                    if is_retryable_status(status) {
                        if attempt < max {
                            let delay = retry_after
                                .unwrap_or_else(|| backoff_delay(&self.retry_config, attempt));
                            tokio::time::sleep(delay).await;
                            continue;
                        }
                        if max > 0 {
                            return Err(Error::Exhausted {
                                attempts: max + 1,
                                source: Box::new(Self::into_api_error(response).await),
                            });
                        }
                    }

                    return Ok(response);
                }
                Err(e) => {
                    if is_retryable_error(&e) && attempt < max {
                        let delay = backoff_delay(&self.retry_config, attempt);
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    if attempt > 0 {
                        return Err(Error::Exhausted {
                            attempts: attempt + 1,
                            source: Box::new(Error::from(e)),
                        });
                    }
                    return Err(Error::from(e));
                }
            }
        }

        unreachable!("loop always returns")
    }

    /// Send a single request without retry (for non-idempotent requests).
    ///
    /// Still respects the client-side rate limiter.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Http`] on network failure, or the raw response
    /// for the caller to inspect.
    pub(crate) async fn send_once(&self, request: RequestBuilder) -> Result<Response, Error> {
        self.rate_limiter.wait().await;

        let response = request.send().await?;

        if response.status() == StatusCode::TOO_MANY_REQUESTS
            && let Some(retry_after) = parse_retry_after(&response)
        {
            self.rate_limiter.record(retry_after).await;
        }

        Ok(response)
    }

    /// Send a request and deserialize the response envelope (with retry).
    pub(crate) async fn send_envelope<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<ApiResponse<T>, Error> {
        let response = self.send_with_retry(request).await?;
        Self::parse_envelope(response).await
    }

    /// Send a non-retryable request and deserialize the response envelope.
    pub(crate) async fn send_envelope_once<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<ApiResponse<T>, Error> {
        let response = self.send_once(request).await?;
        Self::parse_envelope(response).await
    }

    /// Parse a response into the API envelope.
    async fn parse_envelope<T: DeserializeOwned>(
        response: Response,
    ) -> Result<ApiResponse<T>, Error> {
        if !response.status().is_success() {
            return Err(Self::into_api_error(response).await);
        }

        let bytes = response.bytes().await?;
        from_slice::<ApiResponse<T>>(&bytes)
            .map_err(|e| Error::Deserialize(format!("{e}: {}", String::from_utf8_lossy(&bytes))))
    }

    /// Send a request that returns 204 No Content (with retry).
    pub(crate) async fn send_no_content(&self, request: RequestBuilder) -> Result<(), Error> {
        let response = self.send_with_retry(request).await?;

        if response.status().is_success() {
            return Ok(());
        }

        Err(Self::into_api_error(response).await)
    }
}
