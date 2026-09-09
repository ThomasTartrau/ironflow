//! HTTP helpers for Grafana API calls.

use ironflow_core::error::OperationError;
use reqwest::{Client, Response};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

/// Send a GET request to the Grafana API.
///
/// # Errors
///
/// Returns [`OperationError::Http`] if the request fails or the response
/// indicates an error.
pub(crate) async fn get<T: DeserializeOwned>(
    client: &Client,
    url: &str,
    token: &str,
) -> Result<T, OperationError> {
    let resp = client
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(reqwest_err)?;
    handle_response(resp).await
}

/// Send a POST request with a JSON body.
///
/// # Errors
///
/// Returns [`OperationError::Http`] if the request fails or the response
/// indicates an error.
pub(crate) async fn post<B: Serialize, T: DeserializeOwned>(
    client: &Client,
    url: &str,
    token: &str,
    body: &B,
) -> Result<T, OperationError> {
    let resp = client
        .post(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .map_err(reqwest_err)?;
    handle_response(resp).await
}

/// Send a PUT request with a JSON body.
///
/// # Errors
///
/// Returns [`OperationError::Http`] if the request fails or the response
/// indicates an error.
pub(crate) async fn put<B: Serialize, T: DeserializeOwned>(
    client: &Client,
    url: &str,
    token: &str,
    body: &B,
) -> Result<T, OperationError> {
    let resp = client
        .put(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .map_err(reqwest_err)?;
    handle_response(resp).await
}

/// Send a PATCH request with a JSON body.
///
/// # Errors
///
/// Returns [`OperationError::Http`] if the request fails or the response
/// indicates an error.
pub(crate) async fn patch<B: Serialize, T: DeserializeOwned>(
    client: &Client,
    url: &str,
    token: &str,
    body: &B,
) -> Result<T, OperationError> {
    let resp = client
        .patch(url)
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .map_err(reqwest_err)?;
    handle_response(resp).await
}

/// Send a DELETE request.
///
/// # Errors
///
/// Returns [`OperationError::Http`] if the request fails or the response
/// indicates an error.
pub(crate) async fn delete(
    client: &Client,
    url: &str,
    token: &str,
) -> Result<Value, OperationError> {
    let resp = client
        .delete(url)
        .bearer_auth(token)
        .send()
        .await
        .map_err(reqwest_err)?;
    handle_response(resp).await
}

/// Serialize a value to [`serde_json::Value`].
///
/// # Errors
///
/// Returns [`OperationError::Deserialize`] if serialization fails.
pub(crate) fn to_value<T: Serialize>(val: &T) -> Result<Value, OperationError> {
    serde_json::to_value(val).map_err(|e| OperationError::deserialize::<T>(e))
}

async fn handle_response<T: DeserializeOwned>(resp: Response) -> Result<T, OperationError> {
    let status = resp.status();
    if !status.is_success() {
        let message = resp
            .text()
            .await
            .unwrap_or_else(|e| format!("(body unreadable: {e})"));
        return Err(OperationError::Http {
            status: Some(status.as_u16()),
            message,
        });
    }

    // 204 No Content (and other empty-body successes): deserialize from
    // `null` instead of attempting to parse an empty byte stream, which
    // would always fail.
    if status == reqwest::StatusCode::NO_CONTENT {
        return serde_json::from_value(Value::Null)
            .map_err(|e| OperationError::deserialize::<T>(e));
    }

    resp.json()
        .await
        .map_err(|e| OperationError::deserialize::<T>(e))
}

fn reqwest_err(e: reqwest::Error) -> OperationError {
    OperationError::Http {
        status: e.status().map(|s| s.as_u16()),
        message: e.to_string(),
    }
}
