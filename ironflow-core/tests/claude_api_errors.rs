//! Claude CLI API errors, replayed from real CLI output.
//!
//! The fixtures under `fixtures/claude_cli/` are the `assistant` and `result`
//! lines printed by the real `claude` CLI (`--output-format stream-json
//! --verbose`), copied verbatim:
//!
//! * `version_too_old_400.jsonl`: Claude Code 2.1.274 asked for
//!   `claude-opus-5-5`, refused by the API.
//! * `model_not_found_404.jsonl`: Claude Code 2.1.284 asked for a model that
//!   does not exist.
//! * `overloaded_529.jsonl`: Claude Code 2.1.284 pointed with
//!   `ANTHROPIC_BASE_URL` at a local server answering 529 `overloaded_error`,
//!   with `CLAUDE_CODE_MAX_RETRIES=0`.
//! * `budget_exceeded.json`: Claude Code 2.1.284 on `claude-opus-5-5` with
//!   `--output-format json --max-budget-usd 0.10`. `is_error` is `true` there
//!   too, with an `error_max_budget_usd` subtype.
//!
//! The CLI exits with code 1 on each of them, so every transport hands the
//! output to `handle_nonzero_exit`, as these tests do.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::operations::agent::Agent;
use ironflow_core::provider::{AgentConfig, AgentProvider, InvokeFuture};
use ironflow_core::providers::claude::common::{handle_nonzero_exit, parse_response};
use ironflow_core::retry::{RetryPolicy, is_retryable};

const VERSION_TOO_OLD: &str = include_str!("fixtures/claude_cli/version_too_old_400.jsonl");
const MODEL_NOT_FOUND: &str = include_str!("fixtures/claude_cli/model_not_found_404.jsonl");
const OVERLOADED: &str = include_str!("fixtures/claude_cli/overloaded_529.jsonl");
const BUDGET_EXCEEDED: &str = include_str!("fixtures/claude_cli/budget_exceeded.json");

#[derive(Deserialize, JsonSchema)]
#[allow(dead_code)]
struct Plan {
    steps: Vec<String>,
}

/// Config of a verbose agent step with structured output, as `agent-implement`.
fn stream_config_with_schema() -> AgentConfig {
    AgentConfig::new("implement the issue")
        .verbose(true)
        .output::<Plan>()
        .into()
}

/// The `result` line alone: what the CLI prints with `--output-format json`.
fn result_line(fixture: &str) -> &str {
    fixture
        .lines()
        .find(|line| line.contains(r#""type":"result""#))
        .expect("fixture has a result line")
}

fn fail(stdout: &str, config: &AgentConfig) -> AgentError {
    match handle_nonzero_exit(1, stdout, "", config, 0, "test") {
        Ok(output) => panic!("an API error must not succeed, got {:?}", output.value),
        Err(err) => err,
    }
}

#[test]
fn claude_code_version_too_old_is_reported_as_api_error() {
    let err = fail(VERSION_TOO_OLD, &stream_config_with_schema());

    match &err {
        AgentError::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(*status, Some(400));
            assert_eq!(code.as_deref(), Some("claude_code_version_too_old"));
            assert_eq!(
                message,
                "API Error: 400 Claude Code 2.1.274 does not support this model; version 2.1.280 or newer is required. Run 'claude update', or update the Claude desktop app, then try again."
            );
        }
        other => {
            panic!("structured_output validation must not run on an API error, got: {other:?}")
        }
    }
    let message = err.to_string();
    assert!(message.contains("claude_code_version_too_old"), "{message}");
    assert!(
        message.contains("version 2.1.280 or newer is required"),
        "{message}"
    );
}

#[test]
fn claude_code_version_too_old_is_not_retried() {
    let err = fail(VERSION_TOO_OLD, &stream_config_with_schema());

    assert!(!is_retryable(&OperationError::Agent(err)));
}

#[test]
fn claude_code_version_too_old_in_json_mode_is_reported_as_api_error() {
    let config: AgentConfig = AgentConfig::new("implement the issue")
        .output::<Plan>()
        .into();
    let err = fail(result_line(VERSION_TOO_OLD), &config);

    let message = err.to_string();
    assert!(message.contains("claude_code_version_too_old"), "{message}");
    assert!(!is_retryable(&OperationError::Agent(err)));
}

#[test]
fn overloaded_529_is_reported_as_api_error_and_retried() {
    let err = fail(OVERLOADED, &stream_config_with_schema());

    let message = err.to_string();
    assert!(message.contains("529"), "{message}");
    assert!(message.contains("API Error: 529 Overloaded"), "{message}");
    assert!(
        matches!(
            err,
            AgentError::Api {
                status: Some(529),
                code: None,
                ..
            }
        ),
        "got: {err:?}"
    );
    assert!(is_retryable(&OperationError::Agent(err)));
}

#[test]
fn model_not_found_404_is_not_retried() {
    let err = fail(MODEL_NOT_FOUND, &stream_config_with_schema());

    let message = err.to_string();
    assert!(message.contains("404"), "{message}");
    assert!(message.contains("claude-nonexistent-9"), "{message}");
    assert!(
        matches!(
            err,
            AgentError::Api {
                status: Some(404),
                ..
            }
        ),
        "got: {err:?}"
    );
    assert!(!is_retryable(&OperationError::Agent(err)));
}

/// Provider standing in for the `claude` process only: it hands the recorded
/// CLI output to the same `handle_nonzero_exit` every transport calls on exit
/// code 1, and counts how many times the agent was invoked.
struct ReplayCliExit {
    stdout: &'static str,
    calls: AtomicU32,
}

impl AgentProvider for ReplayCliExit {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move { handle_nonzero_exit(1, self.stdout, "", config, 0, "replay") })
    }
}

async fn agent_calls_for(stdout: &'static str) -> u32 {
    let provider = ReplayCliExit {
        stdout,
        calls: AtomicU32::new(0),
    };
    // A structured-output agent gets 2 automatic retries: the step that
    // burned its retry budget on the 400 in the issue.
    let result = Agent::new()
        .prompt("implement the issue")
        .verbose()
        .output_schema_raw(r#"{"type":"object"}"#)
        .retry_policy(RetryPolicy::new(2).backoff(Duration::from_millis(1)))
        .run(&provider)
        .await;
    assert!(result.is_err());
    provider.calls.load(Ordering::SeqCst)
}

#[tokio::test]
async fn claude_code_version_too_old_agent_is_invoked_once() {
    assert_eq!(agent_calls_for(VERSION_TOO_OLD).await, 1);
}

#[tokio::test]
async fn overloaded_529_agent_is_retried() {
    // 1 initial attempt + 2 retries.
    assert_eq!(agent_calls_for(OVERLOADED).await, 3);
}

#[test]
fn budget_exhausted_with_is_error_stays_budget_exceeded() {
    let config = AgentConfig::new("Reply with the single word: ok").max_budget_usd(0.10);

    let err = fail(BUDGET_EXCEEDED, &config);

    match &err {
        AgentError::BudgetExceeded { spent_usd, .. } => {
            assert!((spent_usd - 0.44704).abs() < f64::EPSILON);
        }
        other => panic!("expected BudgetExceeded, got {other:?}"),
    }
    assert!(!is_retryable(&OperationError::Agent(err)));
}

// The CLI output below is hand-written: these are shapes the fixtures do not
// cover, checked against the rules of `parse_response`.

#[test]
fn max_turns_subtype_with_is_error_keeps_its_hint() {
    let stdout = r#"{"session_id":"s1","subtype":"error_max_turns","is_error":true,"result":null,"structured_output":null,"usage":{"input_tokens":100,"output_tokens":50},"total_cost_usd":0.10,"duration_ms":2000}"#;
    let config: AgentConfig = AgentConfig::new("test")
        .output_schema_raw(r#"{"type":"object"}"#)
        .into();

    match parse_response(stdout, &config, 0) {
        Err(AgentError::SchemaValidation { got, .. }) => {
            assert!(got.contains("max turns reached"), "{got}");
        }
        other => panic!("expected SchemaValidation, got {other:?}"),
    }
}

#[test]
fn api_error_without_status_is_retried() {
    let stdout = r#"{"type":"result","subtype":"success","is_error":true,"result":"API Error: Connection error.","total_cost_usd":0,"duration_ms":10}"#;
    let config = AgentConfig::new("test");

    let err = fail(stdout, &config);

    match &err {
        AgentError::Api {
            status,
            code,
            message,
        } => {
            assert_eq!(*status, None);
            assert_eq!(*code, None);
            assert_eq!(message, "API Error: Connection error.");
        }
        other => panic!("expected Api, got {other:?}"),
    }
    assert!(is_retryable(&OperationError::Agent(err)));
}

#[test]
fn api_error_with_null_result_has_placeholder_message() {
    let stdout = r#"{"subtype":"success","is_error":true,"api_error_status":500,"result":null}"#;
    let config = AgentConfig::new("test");

    match parse_response(stdout, &config, 0) {
        Err(AgentError::Api {
            status, message, ..
        }) => {
            assert_eq!(status, Some(500));
            assert_eq!(message, "(no message from the claude CLI)");
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[test]
fn is_error_false_is_a_normal_answer() {
    let stdout = r#"{"subtype":"success","is_error":false,"result":"Hello!","duration_ms":10}"#;
    let config = AgentConfig::new("test");

    let output = parse_response(stdout, &config, 0).expect("not an API error");

    assert_eq!(output.value, json!("Hello!"));
}

#[test]
fn api_error_without_schema_is_not_a_success() {
    let config = AgentConfig::new("implement the issue").verbose(true);
    let err = fail(VERSION_TOO_OLD, &config);

    assert!(
        err.to_string().contains("claude_code_version_too_old"),
        "{err}"
    );
    assert!(!is_retryable(&OperationError::Agent(err)));
}
