//! `AgentConfig::tool_profile`, its serialized form and the retry
//! classification of the tool profile errors.

use serde_json::{from_value, json, to_value};

use ironflow_core::error::{AgentError, OperationError};
use ironflow_core::provider::{AgentConfig, Tool};
use ironflow_core::retry::is_retryable;

use crate::harness::{BUG, SUGGESTION};

#[test]
fn tool_profile_is_not_serialized_when_absent() {
    // A config without a profile must serialize exactly as before the field
    // existed, so the hash keys of recorded fixtures do not change.
    let value = to_value(AgentConfig::new("hi")).expect("serialize");
    assert!(value.get("tool_profile").is_none());
}

#[test]
fn tool_profile_roundtrips_through_serde() {
    let config: AgentConfig = AgentConfig::new("triage").tool_profile(BUG).into();
    let value = to_value(&config).expect("serialize");
    assert_eq!(value["tool_profile"], json!("bug"));

    let back: AgentConfig = from_value(value).expect("deserialize");
    assert_eq!(back.tool_profile, Some(BUG));
}

#[test]
fn tool_profile_deserializes_as_absent_from_older_payloads() {
    let back: AgentConfig = from_value(json!({"prompt": "hi"})).expect("deserialize");
    assert_eq!(back.tool_profile, None);
}

#[test]
fn tool_profile_empty_name_is_rejected_on_deserialize() {
    let err = from_value::<AgentConfig>(json!({"prompt": "hi", "tool_profile": ""}))
        .expect_err("an empty profile name is invalid");
    assert_eq!(err.to_string(), "tool profile name must not be empty");
}

#[test]
fn tool_profile_last_call_wins() {
    let config = AgentConfig::new("x")
        .tool_profile(SUGGESTION)
        .tool_profile(BUG);
    assert_eq!(config.tool_profile, Some(BUG));
}

#[test]
fn tool_profile_combines_with_allow_tool() {
    let config = AgentConfig::new("x")
        .allow_tool(Tool::Read)
        .tool_profile(BUG);
    assert_eq!(config.tool_profile, Some(BUG));
    assert_eq!(config.allowed_tools, vec!["Read"]);
}

#[test]
fn tool_profile_errors_are_not_retryable() {
    let unknown = OperationError::Agent(AgentError::UnknownToolProfile {
        profile: "bug".to_string(),
        available: Vec::new(),
    });
    let unsupported = OperationError::Agent(AgentError::ToolProfileUnsupported {
        provider: "claude-code".to_string(),
        profile: "bug".to_string(),
    });
    assert!(!is_retryable(&unknown));
    assert!(!is_retryable(&unsupported));
}
