//! HTTP response helpers for mapping Tempo API errors to [`OperationError`].

use ironflow_core::error::OperationError;
use reqwest::{RequestBuilder, Response};
use serde_json::{Value, from_slice};

/// Validate that a string is safe to use as a single URL path segment.
///
/// Rejects values containing `/`, `\`, `..`, `?`, `#`, or that are empty.
/// This prevents path traversal and query-string injection when the value
/// is interpolated into a URL via `format!()`.
///
/// # Errors
///
/// Returns [`OperationError::External`] with origin `"tempo"` if the value
/// is not a valid path segment.
pub(crate) fn validate_path_segment(value: &str, name: &str) -> Result<(), OperationError> {
    if value.is_empty()
        || value.contains("..")
        || !value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
    {
        return Err(OperationError::External {
            origin: "tempo".into(),
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
pub(crate) async fn check_response(response: Response) -> Result<Vec<u8>, OperationError> {
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
pub(crate) async fn send_request(
    req: RequestBuilder,
    label: &str,
) -> Result<Response, OperationError> {
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
pub(crate) fn parse_json_body(body: &[u8]) -> Result<Value, OperationError> {
    from_slice(body).map_err(|e| OperationError::Deserialize {
        target_type: "Value".into(),
        reason: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use reqwest::Client;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

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

    #[test]
    fn validate_path_segment_accepts_valid_names() {
        assert!(validate_path_segment("abc123", "id").is_ok());
        assert!(validate_path_segment("trace-id-123", "id").is_ok());
        assert!(validate_path_segment("my_trace", "id").is_ok());
        assert!(validate_path_segment("v1.2.3", "tag").is_ok());
    }

    #[test]
    fn validate_path_segment_rejects_empty() {
        assert!(validate_path_segment("", "id").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_slash() {
        assert!(validate_path_segment("a/b", "id").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_backslash() {
        assert!(validate_path_segment("a\\b", "id").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_dot_dot() {
        assert!(validate_path_segment("..", "id").is_err());
        assert!(validate_path_segment("../admin", "id").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_question_mark() {
        assert!(validate_path_segment("a?b=1", "id").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_hash() {
        assert!(validate_path_segment("a#frag", "id").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_percent_encoded() {
        assert!(validate_path_segment("%2e%2e", "id").is_err());
        assert!(validate_path_segment("foo%2Fbar", "id").is_err());
    }

    #[test]
    fn validate_path_segment_rejects_spaces() {
        assert!(validate_path_segment("a b", "id").is_err());
    }
}
