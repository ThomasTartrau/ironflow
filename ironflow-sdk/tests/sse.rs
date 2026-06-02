//! Tests for SSE event parsing.

use ironflow_sdk::sse::SseEvent;

#[test]
fn sse_event_deserialize() {
    let event = SseEvent {
        event_type: "test".to_string(),
        data: r#"{"value": 42}"#.to_string(),
    };

    #[derive(serde::Deserialize)]
    struct Payload {
        value: i32,
    }

    let payload: Payload = event.deserialize().unwrap();
    assert_eq!(payload.value, 42);
}

#[test]
fn sse_event_deserialize_error() {
    let event = SseEvent {
        event_type: "test".to_string(),
        data: "not json".to_string(),
    };

    let result = event.deserialize::<serde_json::Value>();
    assert!(result.is_err());
}
