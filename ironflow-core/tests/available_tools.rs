//! `AgentConfig::available_tools` reaches the Claude CLI as `--tools`, the
//! set of built-in tools the session offers. Unlike `--allowedTools`, which
//! only pre-approves, a tool left out of `--tools` cannot be called at all:
//! in `dontAsk`, Claude Code still accepts a read-only `Bash` or `Monitor`
//! command missing from `--allowedTools`.

use ironflow_core::provider::{AgentConfig, Tool};
use ironflow_core::providers::claude::common::{build_args, build_command};
use serde_json::{from_value, json, to_value};

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let idx = args.iter().position(|a| a == flag)?;
    args.get(idx + 1).map(String::as_str)
}

#[test]
fn build_args_passes_the_available_tools() {
    let config = AgentConfig::new("review").available_tools([Tool::Read, Tool::Bash]);

    let args = build_args(&config).unwrap();

    assert_eq!(value_after(&args, "--tools"), Some("Read,Bash"));
}

#[test]
fn build_command_passes_the_available_tools() {
    let config =
        AgentConfig::new("review").available_tools([Tool::Read, Tool::Custom("Skill".to_string())]);

    let built = build_command(&config).unwrap();

    assert_eq!(value_after(&built.args, "--tools"), Some("Read,Skill"));
}

#[test]
fn no_tools_flag_without_available_tools() {
    let config = AgentConfig::new("review");

    let args = build_args(&config).unwrap();
    let built = build_command(&config).unwrap();

    assert!(!args.iter().any(|a| a == "--tools"), "{args:?}");
    assert!(
        !built.args.iter().any(|a| a == "--tools"),
        "{:?}",
        built.args
    );
}

/// An empty list offers no built-in tool: `--tools ""`, not the default set.
#[test]
fn empty_available_tools_offers_no_builtin_tool() {
    let config = AgentConfig::new("review").available_tools([]);

    let args = build_args(&config).unwrap();
    let built = build_command(&config).unwrap();

    assert_eq!(value_after(&args, "--tools"), Some(""));
    assert_eq!(value_after(&built.args, "--tools"), Some(""));
}

/// The list survives a serde round trip, and a config stored before the
/// field existed still deserializes, to the default set.
#[test]
fn available_tools_round_trips_and_defaults_to_none() {
    let config = AgentConfig::new("review").available_tools([Tool::Read]);

    let value = to_value(&config).unwrap();
    let back: AgentConfig = from_value(value.clone()).unwrap();
    let old: AgentConfig = from_value(json!({ "prompt": "review" })).unwrap();

    assert_eq!(value["available_tools"], json!(["Read"]));
    assert_eq!(back.available_tools, Some(vec!["Read".to_string()]));
    assert_eq!(old.available_tools, None);
    assert!(to_value(&old).unwrap().get("available_tools").is_none());
}
