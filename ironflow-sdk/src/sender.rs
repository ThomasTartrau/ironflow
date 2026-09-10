//! Retry-aware request sending for [`IronflowClient`].

use ironflow_types::ApiResponse;
use reqwest::{RequestBuilder, StatusCode};
use serde::de::DeserializeOwned;

use crate::client::IronflowClient;
use crate::error::Error;
use crate::retry::{backoff_delay, is_retryable_error, is_retryable_status, parse_retry_after};

impl IronflowClient {
    /// Send a request with automatic retry on transient failures.
    ///
    /// Retries on 429, 502, 503, 504 and connection/timeout errors.
    /// Respects the `Retry-After` header on 429 responses.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Exhausted`] when all retry attempts are used up.
    /// Returns the underlying [`Error::Http`] or [`Error::Api`] on
    /// non-retryable failures.
    pub(crate) async fn send_with_retry(
        &self,
        request: RequestBuilder,
    ) -> Result<reqwest::Response, Error> {
        self.rate_limiter.wait().await;

        let max = self.retry_config.max_retries;
        let mut last_error: Option<Error> = None;

        for attempt in 0..=max {
            let req = request
                .try_clone()
                .expect("request body must be cloneable for retries");

            match req.send().await {
                Ok(response) => {
                    let status = response.status();
                    if status == StatusCode::TOO_MANY_REQUESTS
                        && let Some(retry_after) = parse_retry_after(&response)
                    {
                        self.rate_limiter.record(retry_after).await;
                    }
                    if is_retryable_status(status) && attempt < max {
                        let delay = if status == StatusCode::TOO_MANY_REQUESTS {
                            parse_retry_after(&response)
                                .unwrap_or_else(|| backoff_delay(&self.retry_config, attempt))
                        } else {
                            backoff_delay(&self.retry_config, attempt)
                        };
                        last_error = Some(Self::into_api_error(response).await);
                        tokio::time::sleep(delay).await;
                        continue;
                    }
                    return Ok(response);
                }
                Err(e) => {
                    if is_retryable_error(&e) && attempt < max {
                        let delay = backoff_delay(&self.retry_config, attempt);
                        last_error = Some(Error::from(e));
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

        Err(Error::Exhausted {
            attempts: max + 1,
            source: Box::new(last_error.expect("at least one attempt was made")),
        })
    }

    /// Send a request and deserialize the response envelope.
    pub(crate) async fn send_envelope<T: DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<ApiResponse<T>, Error> {
        let response = self.send_with_retry(request).await?;

        if !response.status().is_success() {
            return Err(Self::into_api_error(response).await);
        }

        let bytes = response.bytes().await?;
        serde_json::from_slice::<ApiResponse<T>>(&bytes)
            .map_err(|e| Error::Deserialize(format!("{e}: {}", String::from_utf8_lossy(&bytes))))
    }

    /// Send a request that returns 204 No Content.
    pub(crate) async fn send_no_content(&self, request: RequestBuilder) -> Result<(), Error> {
        let response = self.send_with_retry(request).await?;

        if response.status().is_success() {
            return Ok(());
        }

        Err(Self::into_api_error(response).await)
    }
}
