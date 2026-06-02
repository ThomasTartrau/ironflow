//! Tests for SDK error types.

use ironflow_sdk::error::Error;
use ironflow_types::ErrorEnvelope;

#[test]
fn deserialize_error_envelope() {
    let json = r#"{"code": "RUN_NOT_FOUND", "message": "run not found"}"#;
    let err: ErrorEnvelope = serde_json::from_str(json).unwrap();
    assert_eq!(err.code, "RUN_NOT_FOUND");
    assert_eq!(err.message, "run not found");
}

#[test]
fn error_api_constructor() {
    let err = Error::api(404, "RUN_NOT_FOUND", "run not found");
    assert!(err.is_api_error());
    assert_eq!(err.status(), Some(404));
    assert_eq!(err.code(), Some("RUN_NOT_FOUND"));
}

#[test]
fn error_http_is_not_api_error() {
    let err = Error::Sse("connection lost".to_string());
    assert!(!err.is_api_error());
    assert!(err.status().is_none());
    assert!(err.code().is_none());
}

#[test]
fn error_display() {
    let err = Error::api(404, "RUN_NOT_FOUND", "run not found");
    let display = format!("{err}");
    assert!(display.contains("404"));
    assert!(display.contains("RUN_NOT_FOUND"));
    assert!(display.contains("run not found"));
}
