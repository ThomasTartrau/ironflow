//! Standard response types and helpers for the REST API.
//!
//! All successful responses are wrapped in the [`ApiResponse`] envelope
//! with optional pagination metadata.

use axum::Json;
use serde::Serialize;

pub use ironflow_types::{ApiMeta, ApiResponse, ErrorEnvelope};

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
    fn ok_wraps_data_without_meta() {
        let Json(response) = ok(vec![1, 2, 3]);
        let json_val = serde_json::to_value(&response).expect("serialize");
        assert_eq!(json_val["data"], json!([1, 2, 3]));
        assert_eq!(json_val["meta"], json!(null));
    }

    #[test]
    fn ok_paged_wraps_data_with_pagination() {
        let Json(response) = ok_paged(vec!["a", "b"], 2, 20, 100);
        let json_val = serde_json::to_value(&response).expect("serialize");
        assert_eq!(json_val["data"], json!(["a", "b"]));
        assert_eq!(json_val["meta"]["page"], 2);
        assert_eq!(json_val["meta"]["per_page"], 20);
        assert_eq!(json_val["meta"]["total"], 100);
    }
}
