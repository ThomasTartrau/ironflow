//! Standard response types and helpers for the REST API.
//!
//! All successful responses are wrapped in the [`ApiResponse`] envelope
//! with optional pagination metadata.

use axum::Json;
use serde::Serialize;

/// Pagination metadata.
///
/// Included in the response when the endpoint returns paginated results.
///
/// # Examples
///
/// ```
/// use ironflow_api::response::ApiMeta;
///
/// let meta = ApiMeta {
///     page: Some(1),
///     per_page: Some(20),
///     total: Some(100),
/// };
/// assert_eq!(meta.page, Some(1));
/// ```
#[derive(Debug, Serialize, Clone)]
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
        ApiMeta {
            page: None,
            per_page: None,
            total: None,
        }
    }

    /// Create pagination metadata.
    pub fn paginated(page: u32, per_page: u32, total: u64) -> Self {
        ApiMeta {
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
/// use ironflow_api::response::ApiResponse;
///
/// let response = ApiResponse {
///     data: "hello".to_string(),
///     meta: None,
/// };
/// assert_eq!(response.data, "hello");
/// ```
#[derive(Debug, Serialize)]
pub struct ApiResponse<T: Serialize> {
    /// The response payload.
    pub data: T,
    /// Optional pagination metadata.
    pub meta: Option<ApiMeta>,
}

/// Helper to wrap data in a successful response without pagination.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::response::ok;
///
/// # async fn handler() {
/// let data = vec!["a", "b"];
/// let response = ok(data);
/// // Returns: { "data": ["a", "b"] }
/// # }
/// ```
pub fn ok<T: Serialize>(data: T) -> Json<ApiResponse<T>> {
    Json(ApiResponse { data, meta: None })
}

/// Helper to wrap paginated data in a successful response.
///
/// # Examples
///
/// ```no_run
/// use ironflow_api::response::ok_paged;
///
/// # async fn handler() {
/// let data = vec!["a", "b"];
/// let response = ok_paged(data, 1, 20, 100);
/// // Returns: { "data": ["a", "b"], "meta": { "page": 1, "per_page": 20, "total": 100 } }
/// # }
/// ```
pub fn ok_paged<T: Serialize>(
    data: T,
    page: u32,
    per_page: u32,
    total: u64,
) -> Json<ApiResponse<T>> {
    Json(ApiResponse {
        data,
        meta: Some(ApiMeta::paginated(page, per_page, total)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn api_meta_empty() {
        let meta = ApiMeta::empty();
        assert_eq!(meta.page, None);
        assert_eq!(meta.per_page, None);
        assert_eq!(meta.total, None);
    }

    #[test]
    fn api_meta_paginated() {
        let meta = ApiMeta::paginated(2, 50, 200);
        assert_eq!(meta.page, Some(2));
        assert_eq!(meta.per_page, Some(50));
        assert_eq!(meta.total, Some(200));
    }

    #[test]
    fn api_response_serialize() {
        let response = ApiResponse {
            data: vec![1, 2, 3],
            meta: None,
        };
        let json_val = serde_json::to_value(&response).expect("serialize");
        assert_eq!(json_val["data"], json!([1, 2, 3]));
        assert_eq!(json_val["meta"], serde_json::json!(null));
    }

    #[test]
    fn api_response_with_meta_serialize() {
        let response = ApiResponse {
            data: vec!["a"],
            meta: Some(ApiMeta::paginated(1, 10, 50)),
        };
        let json_val = serde_json::to_value(&response).expect("serialize");
        assert_eq!(json_val["data"], json!(["a"]));
        assert_eq!(json_val["meta"]["page"], 1);
        assert_eq!(json_val["meta"]["per_page"], 10);
        assert_eq!(json_val["meta"]["total"], 50);
    }
}
