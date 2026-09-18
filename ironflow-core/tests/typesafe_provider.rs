//! Integration test for the TypeSafe (Jev) decision provider HTTP client.
//!
//! Runs against a real local TCP server (no mock): the server captures the raw
//! request so we can assert the wire format and Bearer auth, then replies with a
//! canned System One response.
#![cfg(feature = "provider-typesafe")]

use std::collections::BTreeMap;

use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

use ironflow_core::decision::{DecisionProvider, DecisionQuestion, DecisionRequest, NoulCriteria};
use ironflow_core::error::AgentError;
use ironflow_core::providers::http::TypeSafeProvider;

/// Read one HTTP request (headers + body via Content-Length) from the stream.
async fn read_request(stream: &mut tokio::net::TcpStream) -> String {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        let n = stream.read(&mut chunk).await.unwrap();
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        let text = String::from_utf8_lossy(&buf);
        if let Some(header_end) = text.find("\r\n\r\n") {
            let headers = &text[..header_end];
            let content_length = headers
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            if buf.len() >= body_start + content_length {
                break;
            }
        }
    }
    String::from_utf8_lossy(&buf).to_string()
}

/// Spawn a one-shot server that replies with `status`/`body` and returns the
/// captured request text via a channel. Returns the base URL.
async fn spawn_server(
    status_line: &'static str,
    body: &'static str,
) -> (String, oneshot::Receiver<String>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel();
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let request = read_request(&mut stream).await;
        let response = format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.flush().await.unwrap();
        let _ = tx.send(request);
    });
    (format!("http://{addr}/v1/systemone"), rx)
}

fn sample_request() -> DecisionRequest {
    let mut questions = BTreeMap::new();
    questions.insert(
        "is_urgent".to_string(),
        DecisionQuestion::Noul {
            instructions: json!("Does this convey urgency?"),
            criteria: NoulCriteria {
                if_true: Some("time-sensitive".to_string()),
                if_false: None,
            },
        },
    );
    questions.insert(
        "department".to_string(),
        DecisionQuestion::Choice {
            instructions: json!("Which team?"),
            criteria: BTreeMap::from([
                ("billing".to_string(), Some("payments".to_string())),
                ("technical".to_string(), Some("bugs".to_string())),
            ]),
        },
    );
    DecisionRequest {
        state: json!("Help! My payouts have been failing for 3 days."),
        model: "jev-latest".into(),
        questions,
    }
}

#[tokio::test]
async fn posts_wire_format_and_parses_typed_response() {
    let response_body = r#"{
        "model": "jev-latest",
        "answers": {
            "is_urgent": { "type": "noul", "noul": 0.92 },
            "department": {
                "type": "choice",
                "choice": "technical",
                "probabilities": { "billing": 0.08, "technical": 0.85, "sales": 0.07 },
                "confidence": 0.82
            }
        },
        "usage": { "input_tokens": 312, "output_tokens": 0 }
    }"#;
    let (url, rx) = spawn_server("HTTP/1.1 200 OK", response_body).await;

    let provider = TypeSafeProvider::new("sk-test-key").with_endpoint(url);
    let output = provider.decide(&sample_request()).await.unwrap();

    // Parsed typed response.
    assert_eq!(output.noul("is_urgent").unwrap(), 0.92);
    assert_eq!(output.choice("department").unwrap().choice, "technical");
    assert_eq!(output.usage.input_tokens, 312);

    // Wire format sent to the server.
    let request = rx.await.unwrap();
    assert!(request.starts_with("POST /v1/systemone HTTP/1.1"));
    assert!(request.contains("authorization: Bearer sk-test-key"));
    let body_start = request.find("\r\n\r\n").unwrap() + 4;
    let body: Value = serde_json::from_str(&request[body_start..]).unwrap();
    assert_eq!(body["model"], "jev-latest");
    assert_eq!(body["questions"]["is_urgent"]["type"], "noul");
    assert_eq!(
        body["questions"]["is_urgent"]["criteria"]["true"],
        "time-sensitive"
    );
    assert_eq!(body["questions"]["department"]["type"], "choice");
}

#[tokio::test]
async fn maps_rate_limit_to_typed_error() {
    let (url, _rx) = spawn_server("HTTP/1.1 429 Too Many Requests", "{}").await;
    let provider = TypeSafeProvider::new("sk").with_endpoint(url);
    let err = provider.decide(&sample_request()).await.unwrap_err();
    assert!(matches!(err, AgentError::RateLimited { .. }));
}

#[tokio::test]
async fn maps_server_error_to_http_provider_error() {
    let (url, _rx) = spawn_server("HTTP/1.1 401 Unauthorized", r#"{"error":"bad key"}"#).await;
    let provider = TypeSafeProvider::new("sk").with_endpoint(url);
    let err = provider.decide(&sample_request()).await.unwrap_err();
    match err {
        AgentError::HttpProvider { status_code, .. } => assert_eq!(status_code, 401),
        other => panic!("expected HttpProvider, got {other:?}"),
    }
}

/// The exact triage request the engine e2e tests ask about, mirroring the
/// `DecisionConfig` builder there (empty noul criteria, choice options with no
/// descriptions, ordered score levels).
fn triage_request(model: &str) -> DecisionRequest {
    let mut questions = BTreeMap::new();
    questions.insert(
        "is_urgent".to_string(),
        DecisionQuestion::Noul {
            instructions: json!("Does this convey urgency?"),
            criteria: NoulCriteria::default(),
        },
    );
    questions.insert(
        "department".to_string(),
        DecisionQuestion::Choice {
            instructions: json!("Which team?"),
            criteria: BTreeMap::from([
                ("billing".to_string(), None),
                ("technical".to_string(), None),
                ("sales".to_string(), None),
            ]),
        },
    );
    questions.insert(
        "frustration".to_string(),
        DecisionQuestion::Score {
            instructions: json!("How frustrated?"),
            criteria: vec!["Calm".into(), "Frustrated".into(), "Very angry".into()],
        },
    );
    DecisionRequest {
        state: json!("Help! My payouts have been failing for 3 days."),
        model: model.into(),
        questions,
    }
}

/// Record a real Jev response through OpenRouter into the committed decision
/// fixture consumed by the engine e2e tests.
///
/// Not run in CI: requires `OPENROUTER_API_KEY` and network. Run once to refresh
/// the fixture:
///
/// ```text
/// OPENROUTER_API_KEY=sk-or-... cargo test -p ironflow-core \
///   --features provider-typesafe --test typesafe_provider \
///   record_triage_fixture_from_openrouter -- --ignored --nocapture
/// ```
#[tokio::test]
#[ignore = "hits the real OpenRouter Decisions API; run manually to refresh the fixture"]
async fn record_triage_fixture_from_openrouter() {
    let key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY must be set");
    let model = std::env::var("JEV_MODEL").unwrap_or_else(|_| "typesafe/jev-latest".to_string());

    let provider = TypeSafeProvider::openrouter(key);
    let request = triage_request(&model);
    let output = provider
        .decide(&request)
        .await
        .expect("OpenRouter Decisions call failed");

    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../ironflow-engine/tests/fixtures/decisions/triage-output.json"
    );
    let json = serde_json::to_string_pretty(&output).unwrap();
    std::fs::write(path, &json).unwrap();
    println!("recorded real Jev output to {path}:\n{json}");
}
