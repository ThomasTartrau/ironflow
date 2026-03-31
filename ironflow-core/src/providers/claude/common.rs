//! Shared utilities for all Claude Code transport providers.
//!
//! This module contains command-line argument building, JSON response parsing,
//! and structured-output extraction logic shared across local, SSH, Docker,
//! and Kubernetes transports.

use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::warn;

use crate::error::AgentError;
use crate::operations::agent::PermissionMode;
use crate::provider::{AgentConfig, AgentOutput};

/// Default timeout for a single Claude CLI invocation (5 minutes).
pub const DEFAULT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Parsed JSON output from the `claude` CLI.
#[derive(Deserialize)]
pub struct ClaudeJsonOutput {
    /// Conversation session identifier for resuming multi-turn calls.
    pub session_id: Option<String>,
    /// Response subtype (e.g. `"success"`, `"error_max_budget_usd"`).
    pub subtype: Option<String>,
    /// The model's text response, if any.
    pub result: Option<Value>,
    /// Typed JSON output when a JSON schema was requested.
    pub structured_output: Option<Value>,
    /// Token usage breakdown.
    pub usage: Option<ClaudeUsage>,
    /// Total cost in USD for this invocation.
    pub total_cost_usd: Option<f64>,
    /// Wall-clock duration in milliseconds.
    pub duration_ms: Option<u64>,
    /// Per-model token usage keyed by model identifier.
    #[serde(rename = "modelUsage")]
    pub model_usage: Option<Map<String, Value>>,
}

/// Token usage statistics from the `claude` CLI.
#[derive(Deserialize)]
pub struct ClaudeUsage {
    /// Direct input tokens consumed.
    pub input_tokens: Option<u64>,
    /// Output tokens generated.
    pub output_tokens: Option<u64>,
    /// Tokens used to populate the prompt cache.
    pub cache_creation_input_tokens: Option<u64>,
    /// Tokens served from the prompt cache.
    pub cache_read_input_tokens: Option<u64>,
}

impl ClaudeUsage {
    /// Total input tokens including cache creation and read tokens.
    pub fn total_input_tokens(&self) -> u64 {
        self.input_tokens.unwrap_or(0)
            + self.cache_creation_input_tokens.unwrap_or(0)
            + self.cache_read_input_tokens.unwrap_or(0)
    }

    /// Total output tokens.
    pub fn total_output_tokens(&self) -> u64 {
        self.output_tokens.unwrap_or(0)
    }
}

/// Environment variable names that must be removed before spawning the
/// `claude` CLI to prevent sub-agent mode interference.
///
/// When ironflow runs inside Claude Code (or cmux), the child process inherits
/// variables like `CLAUDE_CODE_ENTRYPOINT`, `CLAUDE_CODE_SUBAGENT_MODEL`, etc.
/// that force degraded/sub-agent behaviour, wrong models, or altered context
/// handling. We strip all `CLAUDE*` vars plus `IRONFLOW_ALLOW_BYPASS`.
///
/// # Examples
///
/// ```no_run
/// # fn example() {
/// let vars = ironflow_core::providers::claude::common::env_vars_to_remove();
/// assert!(vars.contains(&"IRONFLOW_ALLOW_BYPASS".to_string()));
/// # }
/// ```
pub fn env_vars_to_remove() -> Vec<String> {
    collect_vars_to_remove(std::env::vars().map(|(k, _)| k))
}

/// Filter environment variable names, keeping `CLAUDE*` prefixed ones
/// and always including `IRONFLOW_ALLOW_BYPASS`.
fn collect_vars_to_remove(keys: impl Iterator<Item = String>) -> Vec<String> {
    let mut vars: Vec<String> = keys.filter(|key| key.starts_with("CLAUDE")).collect();
    vars.push("IRONFLOW_ALLOW_BYPASS".to_string());
    vars
}

/// Names of `CLAUDE*` env vars to unset in a remote shell command.
///
/// Returns a space-separated list suitable for `unset VAR1 VAR2 ...`.
pub fn env_unset_shell_prefix() -> String {
    let vars = env_vars_to_remove();
    if vars.is_empty() {
        return String::new();
    }
    format!("unset {} 2>/dev/null; ", vars.join(" "))
}

/// Push a CLI flag and its value onto the argument list.
pub fn push_flag(args: &mut Vec<String>, flag: &str, value: &str) {
    args.push(flag.to_string());
    args.push(value.to_string());
}

/// Push a CLI flag and its value onto the argument list, only if the value is `Some`.
pub fn push_opt(args: &mut Vec<String>, flag: &str, value: &Option<impl ToString>) {
    if let Some(v) = value {
        push_flag(args, flag, &v.to_string());
    }
}

/// Build the CLI argument list from an [`AgentConfig`].
///
/// Returns the list of arguments to pass after the `claude` binary name.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] if `BypassPermissions` is requested
/// without the `IRONFLOW_ALLOW_BYPASS=1` environment variable.
pub fn build_args(config: &AgentConfig) -> Result<Vec<String>, AgentError> {
    let mut args: Vec<String> = vec![
        "-p".to_string(),
        config.prompt.clone(),
        "--output-format".to_string(),
        "json".to_string(),
    ];

    push_opt(&mut args, "--system-prompt", &config.system_prompt);
    push_flag(&mut args, "--model", &config.model.to_string());
    if !config.allowed_tools.is_empty() {
        push_flag(&mut args, "--allowedTools", &config.allowed_tools.join(","));
    }
    push_opt(&mut args, "--max-turns", &config.max_turns);
    push_opt(&mut args, "--max-budget-usd", &config.max_budget_usd);
    push_opt(&mut args, "--mcp-config", &config.mcp_config);

    match config.permission_mode {
        PermissionMode::Default => {}
        PermissionMode::Auto => push_flag(&mut args, "--permission-mode", "auto"),
        PermissionMode::DontAsk => push_flag(&mut args, "--permission-mode", "dontAsk"),
        PermissionMode::BypassPermissions => {
            if std::env::var("IRONFLOW_ALLOW_BYPASS").as_deref() != Ok("1") {
                return Err(AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr:
                        "BypassPermissions requires IRONFLOW_ALLOW_BYPASS=1 environment variable"
                            .to_string(),
                });
            }
            warn!(
                "using BypassPermissions: agent will have unrestricted filesystem and shell access"
            );
            args.push("--dangerously-skip-permissions".to_string());
        }
    }

    push_opt(&mut args, "--json-schema", &config.json_schema);

    if let Some(ref session_id) = config.resume_session_id {
        args.push("--resume".to_string());
        args.push(session_id.clone());
    }

    Ok(args)
}

/// Build a single shell command string from the `claude` binary path and arguments.
///
/// Each argument is escaped with single quotes for safe remote execution via `sh -c`.
pub fn build_shell_command(claude_path: &str, args: &[String]) -> String {
    let mut parts = vec![shell_escape(claude_path)];
    for arg in args {
        parts.push(shell_escape(arg));
    }
    parts.join(" ")
}

/// Escape a string for safe inclusion in a single-quoted shell argument.
///
/// Wraps the value in single quotes, escaping any embedded single quotes
/// using the `'\''` idiom.
fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Extract a structured JSON value from a parsed Claude CLI response.
///
/// Prefers `structured_output`; falls back to parsing `result` as JSON
/// (direct parse, code-fence extraction, or brace extraction).
pub fn extract_structured_value(parsed: &ClaudeJsonOutput) -> Option<Value> {
    let from_structured = parsed.structured_output.as_ref().filter(|v| !v.is_null());
    if let Some(v) = from_structured {
        return Some(v.clone());
    }

    let text = parsed.result.as_ref()?.as_str()?;

    if let Ok(v) = serde_json::from_str(text) {
        return Some(v);
    }

    if let Some(start) = text.find("```json") {
        let json_start = start + "```json".len();
        if let Some(end) = text[json_start..].find("```") {
            let json_str = text[json_start..json_start + end].trim();
            if let Ok(v) = serde_json::from_str(json_str) {
                return Some(v);
            }
        }
    }

    let start = text.find('{')?;
    let end = text.rfind('}')?;
    serde_json::from_str(&text[start..=end]).ok()
}

/// Parse raw stdout from the `claude` CLI into an [`AgentOutput`].
///
/// # Errors
///
/// Returns [`AgentError::SchemaValidation`] if the JSON cannot be parsed or
/// if structured output was requested but not present in the response.
pub fn parse_response(
    stdout: &str,
    config: &AgentConfig,
    fallback_duration_ms: u64,
) -> Result<AgentOutput, AgentError> {
    let parsed: ClaudeJsonOutput =
        serde_json::from_str(stdout).map_err(|e| AgentError::SchemaValidation {
            expected: "ClaudeJsonOutput".to_string(),
            got: format!("parse error: {e}"),
        })?;

    let value = if config.json_schema.is_some() {
        extract_structured_value(&parsed).ok_or_else(|| {
            let hint = match parsed.subtype.as_deref() {
                Some("error_max_budget_usd") => {
                    " (budget exceeded before structured output was generated)"
                }
                Some("error_max_turns") => {
                    " (max turns reached before structured output was generated - use max_turns >= 2 with structured output)"
                }
                Some(sub) => {
                    warn!(subtype = sub, "claude returned no structured_output");
                    ""
                }
                None => "",
            };
            AgentError::SchemaValidation {
                expected: "structured_output field".to_string(),
                got: format!("null{hint}"),
            }
        })?
    } else {
        parsed
            .result
            .filter(|v| !v.is_null())
            .unwrap_or_else(|| Value::String(String::new()))
    };

    let model_name = parsed
        .model_usage
        .as_ref()
        .and_then(|m| m.keys().next().cloned());

    Ok(AgentOutput {
        value,
        session_id: parsed.session_id,
        cost_usd: parsed.total_cost_usd,
        input_tokens: parsed.usage.as_ref().map(|u| u.total_input_tokens()),
        output_tokens: parsed.usage.as_ref().map(|u| u.total_output_tokens()),
        model: model_name,
        duration_ms: parsed.duration_ms.unwrap_or(fallback_duration_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserialize_full_claude_json_output() {
        let raw = json!({
            "session_id": "sess-abc123",
            "subtype": "success",
            "result": "Hello, world!",
            "structured_output": null,
            "usage": {
                "input_tokens": 100,
                "output_tokens": 50,
                "cache_creation_input_tokens": 20,
                "cache_read_input_tokens": 30
            },
            "total_cost_usd": 0.042,
            "duration_ms": 1500,
            "modelUsage": {
                "claude-sonnet-4-20250514": {
                    "inputTokens": 100,
                    "outputTokens": 50
                }
            }
        });

        let parsed: ClaudeJsonOutput = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.session_id, Some("sess-abc123".to_string()));
        assert_eq!(parsed.subtype, Some("success".to_string()));
        assert_eq!(
            parsed.result,
            Some(Value::String("Hello, world!".to_string()))
        );
        assert!(parsed.structured_output.is_none());
        assert_eq!(parsed.total_cost_usd, Some(0.042));
        assert_eq!(parsed.duration_ms, Some(1500));

        let usage = parsed.usage.unwrap();
        assert_eq!(usage.total_input_tokens(), 150); // 100 + 20 + 30
        assert_eq!(usage.total_output_tokens(), 50);

        let model_usage = parsed.model_usage.unwrap();
        assert!(model_usage.contains_key("claude-sonnet-4-20250514"));
    }

    #[test]
    fn deserialize_minimal_claude_json_output() {
        let raw = json!({});

        let parsed: ClaudeJsonOutput = serde_json::from_value(raw).unwrap();
        assert!(parsed.session_id.is_none());
        assert!(parsed.subtype.is_none());
        assert!(parsed.result.is_none());
        assert!(parsed.structured_output.is_none());
        assert!(parsed.usage.is_none());
        assert!(parsed.total_cost_usd.is_none());
        assert!(parsed.duration_ms.is_none());
        assert!(parsed.model_usage.is_none());
    }

    #[test]
    fn deserialize_structured_output_response() {
        let raw = json!({
            "session_id": "sess-xyz",
            "subtype": "success",
            "result": null,
            "structured_output": {"score": 9, "summary": "good"},
            "usage": {
                "input_tokens": 200,
                "output_tokens": 80,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0
            },
            "total_cost_usd": 0.08,
            "duration_ms": 3000
        });

        let parsed: ClaudeJsonOutput = serde_json::from_value(raw).unwrap();
        let structured = parsed.structured_output.unwrap();
        assert_eq!(structured["score"], 9);
        assert_eq!(structured["summary"], "good");
    }

    #[test]
    fn deserialize_budget_exceeded_response() {
        let raw = json!({
            "subtype": "error_max_budget_usd",
            "result": null,
            "structured_output": null,
            "total_cost_usd": 0.10,
            "duration_ms": 5000
        });

        let parsed: ClaudeJsonOutput = serde_json::from_value(raw).unwrap();
        assert_eq!(parsed.subtype, Some("error_max_budget_usd".to_string()));
        assert!(parsed.result.is_none());
        assert!(parsed.structured_output.is_none());
    }

    #[test]
    fn claude_usage_with_all_none_tokens() {
        let usage = ClaudeUsage {
            input_tokens: None,
            output_tokens: None,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: None,
        };
        assert_eq!(usage.total_input_tokens(), 0);
        assert_eq!(usage.total_output_tokens(), 0);
    }

    #[test]
    fn claude_usage_sums_cache_tokens() {
        let usage = ClaudeUsage {
            input_tokens: Some(50),
            output_tokens: Some(25),
            cache_creation_input_tokens: Some(10),
            cache_read_input_tokens: Some(15),
        };
        assert_eq!(usage.total_input_tokens(), 75); // 50 + 10 + 15
        assert_eq!(usage.total_output_tokens(), 25);
    }

    #[test]
    fn extract_structured_prefers_structured_output() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": "{\"other\": 1}",
            "structured_output": {"score": 9},
        }))
        .unwrap();
        let v = extract_structured_value(&parsed).unwrap();
        assert_eq!(v["score"], 9);
    }

    #[test]
    fn extract_structured_from_result_direct_parse() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": "{\"score\": 9}",
            "structured_output": null,
        }))
        .unwrap();
        let v = extract_structured_value(&parsed).unwrap();
        assert_eq!(v["score"], 9);
    }

    #[test]
    fn extract_structured_from_code_fence() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": "Here is the result:\n```json\n{\"score\": 9}\n```\nDone.",
            "structured_output": null,
        }))
        .unwrap();
        let v = extract_structured_value(&parsed).unwrap();
        assert_eq!(v["score"], 9);
    }

    #[test]
    fn extract_structured_from_brace_extraction() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": "The answer is {\"score\": 9} as expected.",
            "structured_output": null,
        }))
        .unwrap();
        let v = extract_structured_value(&parsed).unwrap();
        assert_eq!(v["score"], 9);
    }

    #[test]
    fn extract_structured_returns_none_when_both_null() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": null,
            "structured_output": null,
        }))
        .unwrap();
        assert!(extract_structured_value(&parsed).is_none());
    }

    #[test]
    fn extract_structured_returns_none_for_non_json_text() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": "just plain text with no json",
            "structured_output": null,
        }))
        .unwrap();
        assert!(extract_structured_value(&parsed).is_none());
    }

    #[test]
    fn model_name_extracted_from_model_usage() {
        let raw = json!({
            "result": "ok",
            "modelUsage": {
                "claude-opus-4-20250514": {"inputTokens": 100}
            }
        });
        let parsed: ClaudeJsonOutput = serde_json::from_value(raw).unwrap();
        let name = parsed
            .model_usage
            .as_ref()
            .and_then(|m| m.keys().next().cloned());
        assert_eq!(name, Some("claude-opus-4-20250514".to_string()));
    }

    #[test]
    fn build_args_basic_prompt() {
        let config = AgentConfig::new("hello world");
        let args = build_args(&config).unwrap();
        assert_eq!(args[0], "-p");
        assert_eq!(args[1], "hello world");
        assert_eq!(args[2], "--output-format");
        assert_eq!(args[3], "json");
    }

    #[test]
    fn env_vars_to_remove_always_includes_ironflow_allow_bypass() {
        let vars = env_vars_to_remove();
        assert!(
            vars.contains(&"IRONFLOW_ALLOW_BYPASS".to_string()),
            "IRONFLOW_ALLOW_BYPASS must always be removed"
        );
    }

    #[test]
    fn collect_vars_to_remove_captures_claude_prefixed_vars() {
        let keys = vec![
            "CLAUDE_CODE_ENTRYPOINT",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_AUTOCOMPACT_PCT_OVERRIDE",
            "CLAUDECODE",
            "PATH",
            "HOME",
        ];
        let vars = collect_vars_to_remove(keys.into_iter().map(String::from));

        assert!(vars.contains(&"CLAUDE_CODE_ENTRYPOINT".to_string()));
        assert!(vars.contains(&"CLAUDE_CODE_SUBAGENT_MODEL".to_string()));
        assert!(vars.contains(&"CLAUDE_AUTOCOMPACT_PCT_OVERRIDE".to_string()));
        assert!(vars.contains(&"CLAUDECODE".to_string()));
        assert!(vars.contains(&"IRONFLOW_ALLOW_BYPASS".to_string()));
    }

    #[test]
    fn collect_vars_to_remove_excludes_unrelated_vars() {
        let keys = vec!["PATH", "HOME", "RUST_LOG"];
        let vars = collect_vars_to_remove(keys.into_iter().map(String::from));

        assert!(!vars.contains(&"PATH".to_string()));
        assert!(!vars.contains(&"HOME".to_string()));
        // IRONFLOW_ALLOW_BYPASS is always present
        assert_eq!(vars.len(), 1);
    }

    #[test]
    fn env_unset_shell_prefix_format() {
        // env_unset_shell_prefix always includes IRONFLOW_ALLOW_BYPASS at minimum
        let prefix = env_unset_shell_prefix();
        assert!(prefix.starts_with("unset "));
        assert!(prefix.ends_with("2>/dev/null; "));
        assert!(prefix.contains("IRONFLOW_ALLOW_BYPASS"));
    }

    #[test]
    fn build_args_bypass_without_env_fails() {
        let mut config = AgentConfig::new("test");
        config.permission_mode = PermissionMode::BypassPermissions;
        // SAFETY: This test runs single-threaded and only removes a test-specific
        // env var that no other test reads concurrently.
        unsafe { std::env::remove_var("IRONFLOW_ALLOW_BYPASS") };
        let result = build_args(&config);
        assert!(result.is_err());
    }

    #[test]
    fn build_shell_command_escapes_quotes() {
        let args = vec!["-p".to_string(), "it's a test".to_string()];
        let cmd = build_shell_command("claude", &args);
        assert_eq!(cmd, "'claude' '-p' 'it'\\''s a test'");
    }

    #[test]
    fn shell_escape_basic() {
        assert_eq!(shell_escape("hello"), "'hello'");
    }

    #[test]
    fn shell_escape_with_single_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn parse_response_text_mode() {
        let stdout = r#"{"session_id":"s1","result":"Hello","usage":{"input_tokens":10,"output_tokens":5},"total_cost_usd":0.01,"duration_ms":100}"#;
        let config = AgentConfig::new("test");
        let output = parse_response(stdout, &config, 200).unwrap();
        assert_eq!(output.value, Value::String("Hello".to_string()));
        assert_eq!(output.session_id, Some("s1".to_string()));
        assert_eq!(output.duration_ms, 100);
    }

    #[test]
    fn parse_response_uses_fallback_duration() {
        let stdout = r#"{"result":"ok"}"#;
        let config = AgentConfig::new("test");
        let output = parse_response(stdout, &config, 999).unwrap();
        assert_eq!(output.duration_ms, 999);
    }

    #[test]
    fn parse_response_invalid_json() {
        let config = AgentConfig::new("test");
        let result = parse_response("not json", &config, 0);
        assert!(result.is_err());
    }
}
