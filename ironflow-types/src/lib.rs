//! Shared API envelope types for the Ironflow ecosystem.
//!
//! Defines the standard response envelope (`ApiResponse<T>` + `ApiMeta`)
//! and error envelope (`ErrorEnvelope`) used by both the server
//! ([`ironflow-api`]) and the client SDK ([`ironflow-sdk`]).
//!
//! # Features
//!
//! - **`openapi`** -- derive [`utoipa::ToSchema`] for OpenAPI spec generation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Pagination metadata returned by list endpoints.
///
/// # Examples
///
/// ```
/// use ironflow_types::ApiMeta;
///
/// let meta = ApiMeta::paginated(2, 50, 200);
/// assert_eq!(meta.page, Some(2));
/// assert_eq!(meta.total, Some(200));
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMeta {
    /// Current page number (1-based).
    pub page: Option<u32>,
    /// Items per page.
    pub per_page: Option<u32>,
    /// Total number of items matching the filter.
    pub total: Option<u64>,
}

impl ApiMeta {
    /// Create an empty metadata object (no pagination).
    pub fn empty() -> Self {
        Self {
            page: None,
            per_page: None,
            total: None,
        }
    }

    /// Create pagination metadata.
    pub fn paginated(page: u32, per_page: u32, total: u64) -> Self {
        Self {
            page: Some(page),
            per_page: Some(per_page),
            total: Some(total),
        }
    }
}

/// Standard response envelope for all successful API responses.
///
/// Serialized as: `{ "data": ..., "meta": { "page": ..., "total": ... } }`
///
/// # Examples
///
/// ```
/// use ironflow_types::ApiResponse;
///
/// let json = r#"{"data": [1, 2, 3], "meta": null}"#;
/// let resp: ApiResponse<Vec<i32>> = serde_json::from_str(json).unwrap();
/// assert_eq!(resp.data.len(), 3);
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
    /// The response payload.
    pub data: T,
    /// Optional pagination metadata.
    pub meta: Option<ApiMeta>,
}

/// Error response body.
///
/// The inner part of the API error envelope:
/// `{ "error": { "code": "...", "message": "...", "details": { ... } } }`.
///
/// `details` carries error-specific structured context (for example the run
/// holding a conflicting idempotency key). It is omitted from the JSON output
/// when absent.
///
/// # Examples
///
/// ```
/// use ironflow_types::ErrorEnvelope;
///
/// let json = r#"{"code": "RUN_NOT_FOUND", "message": "run not found"}"#;
/// let err: ErrorEnvelope = serde_json::from_str(json).unwrap();
/// assert_eq!(err.code, "RUN_NOT_FOUND");
/// assert!(err.details.is_none());
/// ```
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorEnvelope {
    /// Machine-readable error code (e.g., `RUN_NOT_FOUND`).
    pub code: String,
    /// Human-readable error message.
    pub message: String,
    /// Optional structured context attached to the error.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<std::collections::HashMap<String, serde_json::Value>>))]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn api_meta_empty() {
        let meta = ApiMeta::empty();
        assert!(meta.page.is_none());
        assert!(meta.per_page.is_none());
        assert!(meta.total.is_none());
    }

    #[test]
    fn api_meta_paginated() {
        let meta = ApiMeta::paginated(2, 50, 200);
        assert_eq!(meta.page, Some(2));
        assert_eq!(meta.per_page, Some(50));
        assert_eq!(meta.total, Some(200));
    }

    #[test]
    fn api_response_roundtrip() {
        let response = ApiResponse {
            data: vec![1, 2, 3],
            meta: Some(ApiMeta::paginated(1, 10, 50)),
        };
        let json = serde_json::to_string(&response).unwrap();
        let deserialized: ApiResponse<Vec<i32>> = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.data, vec![1, 2, 3]);
        assert_eq!(deserialized.meta.unwrap().total, Some(50));
    }

    #[test]
    fn error_envelope_roundtrip() {
        let envelope = ErrorEnvelope {
            code: "BAD_REQUEST".to_string(),
            message: "invalid input".to_string(),
            details: None,
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: ErrorEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.code, "BAD_REQUEST");
        assert_eq!(deserialized.message, "invalid input");
        assert!(deserialized.details.is_none());
    }

    #[test]
    fn error_envelope_omits_absent_details() {
        let envelope = ErrorEnvelope {
            code: "BAD_REQUEST".to_string(),
            message: "invalid input".to_string(),
            details: None,
        };
        let json = serde_json::to_string(&envelope).unwrap();
        assert_eq!(json, r#"{"code":"BAD_REQUEST","message":"invalid input"}"#);
    }

    #[test]
    fn error_envelope_roundtrip_with_details() {
        let envelope = ErrorEnvelope {
            code: "IDEMPOTENCY_KEY_CONFLICT".to_string(),
            message: "conflict".to_string(),
            details: Some(json!({ "run_id": "0199-abc" })),
        };
        let json = serde_json::to_string(&envelope).unwrap();
        let deserialized: ErrorEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(
            deserialized.details.expect("details present")["run_id"],
            "0199-abc"
        );
    }

    #[test]
    fn error_envelope_deserializes_without_details_field() {
        let json = r#"{"code": "RUN_NOT_FOUND", "message": "run not found"}"#;
        let envelope: ErrorEnvelope = serde_json::from_str(json).unwrap();
        assert!(envelope.details.is_none());
    }
}
