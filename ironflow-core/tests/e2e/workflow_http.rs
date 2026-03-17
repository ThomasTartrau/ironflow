use std::time::Duration;

use ironflow_core::prelude::*;

#[tokio::test]
async fn http_get_dry_run_returns_synthetic_response() {
    let output = Http::get("https://example.com/api")
        .timeout(Duration::from_secs(10))
        .dry_run(true)
        .await
        .unwrap();
    assert_eq!(output.status(), 200);
    assert!(output.is_success());
    assert_eq!(output.body(), "");
    assert_eq!(output.duration_ms(), 0);
}

#[tokio::test]
async fn http_post_dry_run_with_json_body() {
    let output = Http::post("https://example.com/api")
        .header("Authorization", "Bearer test")
        .json(serde_json::json!({"key": "value"}))
        .dry_run(true)
        .await
        .unwrap();
    assert_eq!(output.status(), 200);
    assert_eq!(output.body(), "");
}

#[tokio::test]
async fn http_connection_refused_returns_error() {
    let result = Http::get("http://127.0.0.1:1")
        .timeout(Duration::from_secs(2))
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(
        matches!(err, ironflow_core::error::OperationError::Http { .. }),
        "expected OperationError::Http, got: {err}"
    );
}

#[tokio::test]
async fn http_output_json_parsing_fails_on_empty_body() {
    let output = Http::get("http://localhost:1").dry_run(true).await.unwrap();
    let result = output.json::<serde_json::Value>();
    assert!(result.is_err());
}

#[tokio::test]
async fn http_tracker_records_step() {
    use ironflow_core::tracker::WorkflowTracker;

    let output = Http::get("http://localhost:1").dry_run(true).await.unwrap();
    let mut tracker = WorkflowTracker::new("http-test");
    tracker.record_http("fetch", &output);
    assert_eq!(tracker.step_count(), 1);
    assert_eq!(tracker.total_cost_usd(), 0.0);
}
