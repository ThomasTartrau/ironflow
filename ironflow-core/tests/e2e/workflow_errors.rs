//! Tests for error paths: shell failure, timeout, agent error propagation, fallback.

use std::fs;

use ironflow_core::prelude::*;
use ironflow_core::providers::record_replay::RecordReplayProvider;

use crate::helpers::*;

#[tokio::test]
async fn shell_failure_stops_workflow() {
    let result = Shell::new("exit 42").await;

    assert!(result.is_err());
    let err = result.unwrap_err();
    match err {
        OperationError::Shell { exit_code, .. } => assert_eq!(exit_code, 42),
        other => panic!("expected Shell error, got: {other}"),
    }
}

#[tokio::test]
async fn shell_timeout_stops_workflow() {
    let result = Shell::new("sleep 10")
        .timeout(std::time::Duration::from_millis(50))
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        OperationError::Timeout { step, .. } => assert_eq!(step, "sleep 10"),
        other => panic!("expected Timeout error, got: {other}"),
    }
}

#[tokio::test]
async fn agent_error_propagates() {
    struct FailingProvider;

    impl ironflow_core::provider::AgentProvider for FailingProvider {
        fn invoke<'a>(
            &'a self,
            _config: &'a ironflow_core::provider::AgentConfig,
        ) -> ironflow_core::provider::InvokeFuture<'a> {
            Box::pin(async move {
                Err(ironflow_core::error::AgentError::ProcessFailed {
                    exit_code: 1,
                    stderr: "simulated agent failure".to_string(),
                })
            })
        }
    }

    let (fixtures_dir, _guard) = temp_fixtures_dir("agent-error");
    fs::create_dir_all(&fixtures_dir).unwrap();

    let provider = RecordReplayProvider::replay(FailingProvider, &fixtures_dir);

    let result = Agent::new()
        .prompt("This will fail")
        .model(Model::Haiku)
        .max_turns(1)
        .max_budget_usd(0.10)
        .run(&provider)
        .await;

    assert!(result.is_err());
    match result.unwrap_err() {
        OperationError::Agent(AgentError::ProcessFailed { exit_code, stderr }) => {
            assert_eq!(exit_code, 1);
            assert!(stderr.contains("simulated"));
        }
        other => panic!("expected Agent(ProcessFailed), got: {other}"),
    }
}

#[tokio::test]
async fn error_recovery_fallback_command() {
    let result = Shell::new("cat /nonexistent_file_xyz_test").await;
    assert!(result.is_err());

    let fallback = Shell::new("echo 'no data available'").await.unwrap();
    assert_eq!(fallback.stdout(), "no data available");
}
