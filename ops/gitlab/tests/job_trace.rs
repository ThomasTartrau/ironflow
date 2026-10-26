use std::sync::Arc;

use gitlab::api::common::NameOrId;
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_gitlab::GitLab;
use ironflow_ops_gitlab::endpoints::jobs::JobTrace;
use serde_json::{Value, json};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn insecure_client(server: &MockServer) -> AsyncGitlab {
    Mock::given(method("GET"))
        .and(path("/api/v4/user"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": 1})))
        .mount(server)
        .await;

    GitlabBuilder::new(server.address().to_string(), "token")
        .insecure()
        .build_async()
        .await
        .unwrap()
}

fn ctx() -> OperationContext {
    OperationContext::new(Arc::new(NoopSecretResolver))
}

fn endpoint() -> JobTrace {
    JobTrace {
        project: NameOrId::from(42),
        job: 7,
    }
}

async fn serve_trace(server: &MockServer, trace: &str) {
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/jobs/7/trace"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw(trace.as_bytes().to_vec(), "text/plain"),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn trace_op_default_view_returns_raw_text() {
    let server = MockServer::start().await;
    let trace = "2024-05-01T10:00:00Z 01O \u{1b}[32mok\u{1b}[0m\r\nline 2\n";
    serve_trace(&server, trace).await;

    let gitlab = GitLab::from(insecure_client(&server).await);
    let result = gitlab.trace_op(endpoint()).execute(&ctx()).await.unwrap();
    assert_eq!(result, Value::String(trace.to_string()));
}

#[tokio::test]
async fn trace_op_decoded_strips_ansi_and_timestamps() {
    let server = MockServer::start().await;
    serve_trace(
        &server,
        "2024-05-01T10:00:00.5Z 01O \u{1b}[32;1m$ cargo test\u{1b}[0;m\n\
         2024-05-01T10:00:01.5Z 01E error: boom\r\n",
    )
    .await;

    let gitlab = GitLab::from(insecure_client(&server).await);
    let result = gitlab
        .trace_op(endpoint())
        .decoded()
        .execute(&ctx())
        .await
        .unwrap();
    assert_eq!(
        result,
        Value::String("$ cargo test\nerror: boom\n".to_string())
    );
}

#[tokio::test]
async fn trace_op_condensed_keeps_failure_and_tail_under_budget() {
    let marker = "[... trace tronquée, début omis ...]";
    let mut lines: Vec<String> = (0..300)
        .map(|i| format!("2024-05-01T10:00:00Z 01O    Compiling dep-{i} v0.1.0"))
        .collect();
    lines.extend(
        (0..100).map(|i| format!("2024-05-01T10:00:00Z 01E warning: unused variable `x{i}`")),
    );
    lines.push("2024-05-01T10:00:01Z 01E error[E0308]: mismatched types".to_string());
    lines.push("2024-05-01T10:00:01Z 01E  --> src/main.rs:10:5".to_string());
    lines.push("2024-05-01T10:00:01Z 01E   |".to_string());
    lines.push("2024-05-01T10:00:01Z 01O ".to_string());
    lines.extend((0..80).map(|i| format!("2024-05-01T10:00:02Z 01O summary line {i}")));
    lines.push(
        "2024-05-01T10:00:03Z 01E error: could not compile `app` due to 1 previous error"
            .to_string(),
    );
    lines.push("2024-05-01T10:00:04Z 00O ERROR: Job failed: exit code 1".to_string());
    let server = MockServer::start().await;
    serve_trace(&server, &lines.join("\n")).await;

    let gitlab = GitLab::from(insecure_client(&server).await);
    let budget = 1500;
    let result = gitlab
        .trace_op(endpoint())
        .condensed(budget, marker)
        .execute(&ctx())
        .await
        .unwrap();
    let text = result.as_str().unwrap();

    assert!(text.chars().count() <= budget);
    assert!(text.contains(marker));
    assert!(text.contains("error[E0308]: mismatched types"));
    assert!(text.contains(" --> src/main.rs:10:5"));
    assert!(text.contains("ERROR: Job failed: exit code 1"));
    assert!(text.contains("summary line 79"));
    assert!(!text.contains("Compiling"));
    assert!(!text.contains("2024-05-01"));
}

#[tokio::test]
async fn trace_op_escapes_group_project_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/group%2Fproject/jobs/7/trace"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(b"hello".to_vec(), "text/plain"))
        .mount(&server)
        .await;

    let gitlab = GitLab::from(insecure_client(&server).await);
    let endpoint = JobTrace {
        project: NameOrId::from("group/project"),
        job: 7,
    };
    let result = gitlab.trace_op(endpoint).execute(&ctx()).await.unwrap();
    assert_eq!(result, Value::String("hello".to_string()));
}

#[tokio::test]
async fn trace_op_404_is_http_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v4/projects/42/jobs/7/trace"))
        .respond_with(ResponseTemplate::new(404).set_body_json(json!({"message": "404 Not found"})))
        .mount(&server)
        .await;

    let gitlab = GitLab::from(insecure_client(&server).await);
    let err = gitlab
        .trace_op(endpoint())
        .execute(&ctx())
        .await
        .unwrap_err();
    assert!(matches!(err, OperationError::Http { .. }), "{err:?}");
}

#[tokio::test]
async fn trace_op_input_reports_endpoint_and_view() {
    let server = MockServer::start().await;
    let gitlab = GitLab::from(insecure_client(&server).await);

    let raw = gitlab.trace_op(endpoint()).input().unwrap();
    assert_eq!(
        raw,
        json!({"endpoint": "projects/42/jobs/7/trace", "view": "raw"})
    );
    let decoded = gitlab.trace_op(endpoint()).decoded().input().unwrap();
    assert_eq!(decoded["view"], "decoded");
    let condensed = gitlab
        .trace_op(endpoint())
        .condensed(500, "[cut]")
        .input()
        .unwrap();
    assert_eq!(
        condensed,
        json!({"endpoint": "projects/42/jobs/7/trace", "view": "condensed", "budget": 500})
    );
}

#[tokio::test]
async fn trace_op_kind_is_gitlab() {
    let server = MockServer::start().await;
    let gitlab = GitLab::from(insecure_client(&server).await);
    assert_eq!(gitlab.trace_op(endpoint()).kind(), "gitlab");
}

#[tokio::test]
async fn trace_op_empty_body_returns_empty_string() {
    let server = MockServer::start().await;
    serve_trace(&server, "").await;

    let gitlab = GitLab::from(insecure_client(&server).await);
    let result = gitlab.trace_op(endpoint()).execute(&ctx()).await.unwrap();
    assert_eq!(result, Value::String(String::new()));
}

#[tokio::test]
#[should_panic(expected = "budget")]
async fn trace_op_condensed_zero_budget_panics() {
    let server = MockServer::start().await;
    let gitlab = GitLab::from(insecure_client(&server).await);
    let _ = gitlab.trace_op(endpoint()).condensed(0, "[cut]");
}
