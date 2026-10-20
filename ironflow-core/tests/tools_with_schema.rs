//! Agent configs with tools and a structured output schema on the same step.
//!
//! `fixtures/claude_cli/tools_with_json_schema.json` is the `result` line
//! printed by the real `claude` CLI 2.1.296 (`--output-format json
//! --allowedTools Read --json-schema ...`, haiku), copied verbatim: the model
//! read `facts.txt` in three turns and `structured_output` holds the answer.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, from_str, from_value, json, to_string};

use ironflow_core::error::AgentError;
use ironflow_core::provider::{AgentConfig, RawSchema, Tool, ToolProfile, WithSchema, WithTools};
use ironflow_core::providers::claude::common::parse_response;

const TOOLS_WITH_SCHEMA: &str = include_str!("fixtures/claude_cli/tools_with_json_schema.json");

/// What the agent reads in `facts.txt`.
#[derive(Debug, PartialEq, Deserialize, JsonSchema)]
struct Facts {
    secret_number: u32,
    owner: String,
}

/// The config of the captured run: a tool and a typed answer.
fn reader() -> AgentConfig {
    AgentConfig::new("Read facts.txt and report its secret_number and owner.")
        .allow_tool(Tool::Read)
        .max_turns(6)
        .output::<Facts>()
        .into()
}

/// The fixture as a CLI hit by claude-code#18536 prints it: `structured_output`
/// null, the answer left in `result`.
fn without_structured_output(result: &str) -> String {
    let mut line: Value = from_str(TOOLS_WITH_SCHEMA).expect("the fixture is JSON");
    line["structured_output"] = Value::Null;
    line["result"] = json!(result);
    to_string(&line).expect("serializable")
}

#[test]
fn tools_and_typed_output_combine_in_either_order() {
    let tools_first: AgentConfig<WithTools, WithSchema<Facts>> = AgentConfig::new("read")
        .allow_tool(Tool::Read)
        .output::<Facts>();
    let schema_first: AgentConfig<WithTools, WithSchema<Facts>> = AgentConfig::new("read")
        .output::<Facts>()
        .allow_tool(Tool::Read);

    for config in [tools_first, schema_first] {
        assert_eq!(config.allowed_tools, vec!["Read"]);
        assert!(
            config
                .json_schema
                .as_deref()
                .is_some_and(|schema| schema.contains("secret_number")),
            "the schema of Facts is kept next to the tools"
        );
    }
}

#[test]
fn a_tool_profile_combines_with_a_raw_schema() {
    const BUG: ToolProfile = ToolProfile::new("bug");

    let config: AgentConfig<WithTools, RawSchema> = AgentConfig::new("triage")
        .output_schema_raw(r#"{"type":"object"}"#)
        .tool_profile(BUG);
    assert_eq!(config.tool_profile, Some(BUG));
    assert_eq!(config.json_schema.as_deref(), Some(r#"{"type":"object"}"#));
}

#[test]
fn tools_with_a_schema_convert_to_the_base_type() {
    let base = reader();
    assert_eq!(base.allowed_tools, vec!["Read"]);
    assert_eq!(base.max_turns, Some(6));
    assert!(base.json_schema.is_some());

    let raw: AgentConfig = AgentConfig::new("grep")
        .allow_tool(Tool::Grep)
        .output_schema_raw(r#"{"type":"object"}"#)
        .into();
    assert_eq!(raw.allowed_tools, vec!["Grep"]);
    assert_eq!(raw.json_schema.as_deref(), Some(r#"{"type":"object"}"#));
}

#[test]
fn the_cli_answers_a_step_with_tools_and_a_schema() {
    let output = parse_response(TOOLS_WITH_SCHEMA, &reader(), 0).expect("a structured answer");
    let facts: Facts = from_value(output.value).expect("the answer matches Facts");
    assert_eq!(
        facts,
        Facts {
            secret_number: 4217,
            owner: "platform-team".to_string(),
        }
    );
}

#[test]
fn a_null_structured_output_falls_back_to_the_result_text() {
    let stdout = without_structured_output(r#"{"secret_number":4217,"owner":"platform-team"}"#);
    let output = parse_response(&stdout, &reader(), 0).expect("the answer is read from result");
    assert_eq!(
        output.value,
        json!({"secret_number": 4217, "owner": "platform-team"})
    );
}

#[test]
fn a_null_structured_output_without_json_is_a_schema_error() {
    let text = "The secret number is 4217, owned by platform-team.";
    let err = parse_response(&without_structured_output(text), &reader(), 0)
        .expect_err("no JSON to extract, never a null answer");
    match err {
        AgentError::SchemaValidation {
            got, raw_response, ..
        } => {
            assert!(got.starts_with("null"), "got: {got}");
            assert_eq!(raw_response.as_deref(), Some(text));
        }
        other => panic!("expected SchemaValidation, got {other:?}"),
    }
}
