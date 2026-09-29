//! `AgentConfig::append_system_prompt` reaches the Claude CLI as
//! `--append-system-prompt`, which extends Claude Code's own system prompt
//! instead of replacing it like `--system-prompt` does.

use ironflow_core::error::AgentError;
use ironflow_core::provider::AgentConfig;
use ironflow_core::providers::claude::common::{build_args, build_command, validate_prompt_size};

fn value_after<'a>(args: &'a [String], flag: &str) -> Option<&'a str> {
    let idx = args.iter().position(|a| a == flag)?;
    args.get(idx + 1).map(String::as_str)
}

#[test]
fn build_args_appends_without_replacing_the_default_system_prompt() {
    let config = AgentConfig::new("review").append_system_prompt("Never hardcode tenants.");

    let args = build_args(&config).unwrap();

    assert_eq!(
        value_after(&args, "--append-system-prompt"),
        Some("Never hardcode tenants.")
    );
    assert!(!args.iter().any(|a| a == "--system-prompt"));
}

#[test]
fn build_command_passes_the_appended_prompt() {
    let config = AgentConfig::new("review").append_system_prompt("rules");

    let built = build_command(&config).unwrap();

    assert_eq!(
        value_after(&built.args, "--append-system-prompt"),
        Some("rules")
    );
}

#[test]
fn both_prompts_are_passed_when_both_are_set() {
    let config = AgentConfig::new("review")
        .system_prompt("You review code.")
        .append_system_prompt("rules");

    let args = build_args(&config).unwrap();

    assert_eq!(
        value_after(&args, "--system-prompt"),
        Some("You review code.")
    );
    assert_eq!(value_after(&args, "--append-system-prompt"), Some("rules"));
}

#[test]
fn no_flag_without_an_appended_prompt() {
    let args = build_args(&AgentConfig::new("review")).unwrap();

    assert!(!args.iter().any(|a| a == "--append-system-prompt"));
}

#[test]
fn prompt_size_counts_the_appended_prompt() {
    let config = AgentConfig::new("x")
        .model("claude-haiku-4-5-20251001")
        .append_system_prompt(&"a".repeat(4_000_000));

    assert!(matches!(
        validate_prompt_size(&config),
        Err(AgentError::PromptTooLarge { .. })
    ));
}

#[test]
fn appended_prompt_survives_serialization_and_is_optional() {
    let config = AgentConfig::new("review").append_system_prompt("rules");
    let json = serde_json::to_value(&config).unwrap();
    assert_eq!(json["append_system_prompt"], "rules");

    let back: AgentConfig = serde_json::from_value(json).unwrap();
    assert_eq!(back.append_system_prompt.as_deref(), Some("rules"));

    // A config stored before the field existed still deserializes.
    let mut legacy = serde_json::to_value(AgentConfig::new("review")).unwrap();
    legacy
        .as_object_mut()
        .unwrap()
        .remove("append_system_prompt");
    let back: AgentConfig = serde_json::from_value(legacy).unwrap();
    assert_eq!(back.append_system_prompt, None);
}
