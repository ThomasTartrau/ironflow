//! Claude CLI providers pick their tools with `allowed_tools` and MCP
//! configuration: a step asking for a tool profile must fail loudly instead of
//! running with tools it did not ask for.

use ironflow_core::error::AgentError;
use ironflow_core::provider::{AgentConfig, AgentProvider};
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::claude::common::build_command;

use crate::harness::{BUG, within_timeout};

fn profiled_config() -> AgentConfig {
    AgentConfig::new("triage").tool_profile(BUG).into()
}

fn assert_unsupported(err: &AgentError) {
    match err {
        AgentError::ToolProfileUnsupported { provider, profile } => {
            assert_eq!(provider, "claude-code");
            assert_eq!(profile, "bug");
        }
        other => panic!("expected ToolProfileUnsupported, got {other:?}"),
    }
    assert_eq!(
        err.to_string(),
        "claude-code does not support tool profiles (step asked for 'bug'): \
         pick its tools with allow_tool or an MCP config"
    );
}

#[test]
fn tool_profile_is_rejected_when_building_the_claude_command() {
    // Every Claude CLI transport (local, Docker, SSH, K8s) builds its command
    // line here before any side effect.
    let err = build_command(&profiled_config()).expect_err("a tool profile must be refused");
    assert_unsupported(&err);
}

#[tokio::test]
async fn tool_profile_is_rejected_by_the_claude_code_provider() {
    within_timeout(async {
        let err = ClaudeCodeProvider::new()
            .invoke(&profiled_config())
            .await
            .expect_err("a tool profile must be refused");
        assert_unsupported(&err);
    })
    .await;
}

#[cfg(feature = "transport-k8s")]
#[tokio::test]
async fn tool_profile_is_rejected_by_the_k8s_ephemeral_provider() {
    use ironflow_core::providers::claude::K8sEphemeralProvider;

    within_timeout(async {
        let err = K8sEphemeralProvider::new("ghcr.io/example/claude:1.0.0")
            .invoke(&profiled_config())
            .await
            .expect_err("a tool profile must be refused before any pod is created");
        assert_unsupported(&err);
    })
    .await;
}
