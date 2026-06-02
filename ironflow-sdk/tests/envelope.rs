//! Tests for the ApiResponse envelope deserialization.

use ironflow_sdk::client::ApiResponse;

#[test]
fn deserialize_envelope_without_meta() {
    let json = r#"{"data": [1, 2, 3], "meta": null}"#;
    let resp: ApiResponse<Vec<i32>> = serde_json::from_str(json).unwrap();
    assert_eq!(resp.data, vec![1, 2, 3]);
    assert!(resp.meta.is_none());
}

#[test]
fn deserialize_envelope_with_meta() {
    let json = r#"{
        "data": ["a", "b"],
        "meta": {"page": 1, "per_page": 20, "total": 100}
    }"#;
    let resp: ApiResponse<Vec<String>> = serde_json::from_str(json).unwrap();
    assert_eq!(resp.data.len(), 2);
    let meta = resp.meta.unwrap();
    assert_eq!(meta.page, Some(1));
    assert_eq!(meta.per_page, Some(20));
    assert_eq!(meta.total, Some(100));
}

#[test]
fn deserialize_envelope_with_partial_meta() {
    let json = r#"{
        "data": "hello",
        "meta": {"page": 1, "per_page": null, "total": null}
    }"#;
    let resp: ApiResponse<String> = serde_json::from_str(json).unwrap();
    assert_eq!(resp.data, "hello");
    let meta = resp.meta.unwrap();
    assert_eq!(meta.page, Some(1));
    assert!(meta.per_page.is_none());
    assert!(meta.total.is_none());
}

#[test]
fn deserialize_envelope_with_object_data() {
    let json = r#"{
        "data": {"name": "deploy", "category": null, "version": null},
        "meta": null
    }"#;
    let resp: ApiResponse<ironflow_sdk::types::WorkflowSummary> =
        serde_json::from_str(json).unwrap();
    assert_eq!(resp.data.name, "deploy");
}
