//! HTTP client for the Ironflow public API.

use std::time::Duration;

use reqwest::Client;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::debug;

use crate::error::McpError;

/// Ironflow API response envelope.
#[derive(Debug, Deserialize)]
struct ApiResponse<T> {
    data: T,
}

/// Ironflow API error envelope.
#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorDetail,
}

/// Detail inside an API error response.
#[derive(Debug, Deserialize)]
struct ApiErrorDetail {
    message: String,
}

/// HTTP client for the Ironflow public REST API.
///
/// Handles authentication via Bearer token and unwraps
/// the standard `{ "data": ..., "meta": ... }` envelope.
pub struct ApiClient {
    client: Client,
    base_url: String,
    token: String,
}

impl ApiClient {
    /// Create a new client with an API key (`irfl_...`).
    pub fn new(base_url: &str, token: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(5))
            .build()
            .expect("failed to build HTTP client");

        Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            token,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}/api/v1{}", self.base_url, path)
    }

    /// Send a GET request and deserialize the `data` field from the response.
    pub async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, McpError> {
        debug!(path, "GET");
        let resp = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(McpError::Http)?;

        self.handle_response(resp).await
    }

    /// Send a POST request with a JSON body.
    pub async fn post<T: DeserializeOwned>(&self, path: &str, body: &Value) -> Result<T, McpError> {
        debug!(path, "POST");
        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(McpError::Http)?;

        self.handle_response(resp).await
    }

    /// Send a POST request without a body, returning raw JSON.
    pub async fn post_action(&self, path: &str) -> Result<Value, McpError> {
        debug!(path, "POST action");
        let resp = self
            .client
            .post(self.url(path))
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(McpError::Http)?;

        self.handle_response(resp).await
    }

    /// Send a GET request with query params and return the full raw JSON
    /// response (the envelope is NOT unwrapped -- includes `data` and `meta`).
    pub async fn get_raw_with_query(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Value, McpError> {
        debug!(path, ?query, "GET raw with query");
        let resp = self
            .client
            .get(self.url(path))
            .bearer_auth(&self.token)
            .query(query)
            .send()
            .await
            .map_err(McpError::Http)?;

        self.handle_raw_response(resp).await
    }

    async fn handle_raw_response(&self, resp: reqwest::Response) -> Result<Value, McpError> {
        let status = resp.status();
        let body = resp.text().await.map_err(McpError::Http)?;

        if !status.is_success() {
            let message = serde_json::from_str::<ApiErrorResponse>(&body)
                .map(|e| e.error.message)
                .unwrap_or(body);
            return Err(McpError::Api {
                status: status.as_u16(),
                message,
            });
        }

        serde_json::from_str(&body).map_err(|e| McpError::Deserialize(format!("{e}: {body}")))
    }

    async fn handle_response<T: DeserializeOwned>(
        &self,
        resp: reqwest::Response,
    ) -> Result<T, McpError> {
        let raw = self.handle_raw_response(resp).await?;
        let api_resp: ApiResponse<T> =
            serde_json::from_value(raw).map_err(|e| McpError::Deserialize(e.to_string()))?;
        Ok(api_resp.data)
    }
}
