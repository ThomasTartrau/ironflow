//! HTTP response helpers and JSON utilities.

use ironflow_core::error::OperationError;
use reqwest::{RequestBuilder, Response, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, from_slice};

/// Validate that a string is safe to use as a single URL path segment.
///
/// Rejects values containing `/`, `\`, `..`, `?`, `#`, or that are empty.
/// This prevents path traversal and query-string injection when the value
/// is interpolated into a URL.
///
/// # Errors
///
/// Returns [`OperationError::External`] with the given `origin` if the
/// value is not a valid path segment.
///
/// # Examples
///
/// ```
/// use ironflow_ops_common::helpers::validate_path_segment;
///
/// assert!(validate_path_segment("abc123", "id", "tempo").is_ok());
/// assert!(validate_path_segment("../admin", "id", "tempo").is_err());
/// ```
pub fn validate_path_segment(value: &str, name: &str, origin: &str) -> Result<(), OperationError> {
    if value.is_empty()
        || value.contains("..")
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(OperationError::External {
            origin: origin.into(),
            message: format!(
                "{name} must be non-empty, must not contain '..', and may only contain ASCII alphanumeric, '-', '_', or '.': {value:?}"
            ),
        });
    }
    Ok(())
}

/// Read the response body and, if the status is not successful, return an
/// [`OperationError::Http`] carrying the status code and the body text.
///
/// On success the raw body bytes are returned for the caller to deserialize.
///
/// # Errors
///
/// Returns [`OperationError::Http`] when:
/// - the HTTP status is not in the 2xx range, or
/// - the response body cannot be read.
pub async fn check_response(response: Response) -> Result<Vec<u8>, OperationError> {
    let status = response.status();
    let status_code = status.as_u16();

    let body = response.bytes().await.map_err(|e| OperationError::Http {
        status: Some(status_code),
        message: format!("failed to read response body: {e}"),
    })?;

    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        return Err(OperationError::Http {
            status: Some(status_code),
            message: text.into_owned(),
        });
    }

    Ok(body.to_vec())
}

/// Send an HTTP request, mapping transport errors to [`OperationError::Http`].
///
/// # Errors
///
/// Returns [`OperationError::Http`] with `status: None` if the request
/// cannot be sent (DNS failure, connection refused, timeout, etc.).
pub async fn send_request(req: RequestBuilder, label: &str) -> Result<Response, OperationError> {
    req.send().await.map_err(|e| OperationError::Http {
        status: None,
        message: format!("{label} request failed: {e}"),
    })
}

/// Deserialize a JSON body into a [`Value`].
///
/// # Errors
///
/// Returns [`OperationError::Deserialize`] if the bytes are not valid JSON.
pub fn parse_json_body(body: &[u8]) -> Result<Value, OperationError> {
    from_slice(body).map_err(|e| OperationError::Deserialize {
        target_type: "Value".into(),
        reason: e.to_string(),
    })
}

/// Check response status and deserialize the JSON body into `T`.
///
/// Handles 204 No Content by deserializing from `null` instead of an
/// empty body, which would always fail.
///
/// # Errors
///
/// Returns [`OperationError::Http`] if the status is not 2xx, or
/// [`OperationError::Deserialize`] if the JSON cannot be parsed.
pub async fn check_response_json<T: DeserializeOwned>(
    response: Response,
) -> Result<T, OperationError> {
    let status = response.status();
    if !status.is_success() {
        let message = response
            .text()
            .await
            .unwrap_or_else(|e| format!("(body unreadable: {e})"));
        return Err(OperationError::Http {
            status: Some(status.as_u16()),
            message,
        });
    }

    if status == StatusCode::NO_CONTENT {
        return serde_json::from_value(Value::Null)
            .map_err(|e| OperationError::deserialize::<T>(e));
    }

    response
        .json()
        .await
        .map_err(|e| OperationError::deserialize::<T>(e))
}

/// Map a [`reqwest::Error`] to [`OperationError::Http`].
pub fn reqwest_err(e: reqwest::Error) -> OperationError {
    OperationError::Http {
        status: e.status().map(|s| s.as_u16()),
        message: e.to_string(),
    }
}

/// Serialize a value to [`serde_json::Value`].
///
/// # Errors
///
/// Returns [`OperationError::Deserialize`] if serialization fails.
pub fn to_value<T: Serialize>(val: &T) -> Result<Value, OperationError> {
    serde_json::to_value(val).map_err(|e| OperationError::deserialize::<T>(e))
}

#[cfg(test)]
mod tests {
    use reqwest::Client;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    // -- Row 3: check_response --

    #[tokio::test]
    async fn check_response_returns_body_on_success() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"status":"success"}"#))
            .mount(&server)
            .await;

        let response = Client::new().get(server.uri()).send().await.unwrap();
        let body = check_response(response).await.unwrap();
        assert_eq!(body, br#"{"status":"success"}"#);
    }

    #[tokio::test]
    async fn check_response_returns_error_on_4xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
            .mount(&server)
            .await;

        let response = Client::new().get(server.uri()).send().await.unwrap();
        let err = check_response(response).await.unwrap_err();
        match err {
            OperationError::Http { status, message } => {
                assert_eq!(status, Some(400));
                assert_eq!(message, "bad request");
            }
            other => panic!("expected Http error, got: {other}"),
        }
    }

    #[tokio::test]
    async fn check_response_returns_error_on_5xx() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal server error"))
            .mount(&server)
            .await;

        let response = Client::new().get(server.uri()).send().await.unwrap();
        let err = check_response(response).await.unwrap_err();
        match err {
            OperationError::Http { status, message } => {
                assert_eq!(status, Some(500));
                assert_eq!(message, "internal server error");
            }
            other => panic!("expected Http error, got: {other}"),
        }
    }

    // -- Row 4: validate_path_segment --

    #[test]
    fn validate_path_segment_accepts_valid_names() {
        assert!(validate_path_segment("abc123", "id", "test").is_ok());
        assert!(validate_path_segment("trace-id-123", "id", "test").is_ok());
        assert!(validate_path_segment("my_trace", "id", "test").is_ok());
        assert!(validate_path_segment("v1.2.3", "tag", "test").is_ok());
    }

    #[test]
    fn validate_path_segment_rejects_empty() {
        assert!(validate_path_segment("", "id", "test").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_slash() {
        assert!(validate_path_segment("a/b", "id", "test").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_backslash() {
        assert!(validate_path_segment("a\\b", "id", "test").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_dot_dot() {
        assert!(validate_path_segment("..", "id", "test").is_err());
        assert!(validate_path_segment("../admin", "id", "test").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_question_mark() {
        assert!(validate_path_segment("a?b=1", "id", "test").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_hash() {
        assert!(validate_path_segment("a#frag", "id", "test").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_percent_encoded() {
        assert!(validate_path_segment("%2e%2e", "id", "test").is_err());
        assert!(validate_path_segment("foo%2Fbar", "id", "test").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_spaces() {
        assert!(validate_path_segment("a b", "id", "test").is_err());
    }

    // -- Row 6: parse_json_body and to_value --

    #[test]
    fn parse_json_body_valid() {
        let body = br#"{"key":"value"}"#;
        let val = parse_json_body(body).unwrap();
        assert_eq!(val["key"], "value");
    }

    #[test]
    fn parse_json_body_invalid() {
        let body = b"not json";
        assert!(parse_json_body(body).is_err());
    }

    #[test]
    fn to_value_serializes() {
        let val = to_value(&"hello").unwrap();
        assert_eq!(val, serde_json::Value::String("hello".into()));
    }
}
