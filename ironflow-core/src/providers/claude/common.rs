//! Shared utilities for all Claude Code transport providers.
//!
//! This module contains command-line argument building, JSON response parsing,
//! and structured-output extraction logic shared across local, SSH, Docker,
//! and Kubernetes transports.

use std::env;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};
use tracing::warn;

use crate::error::AgentError;
use crate::operations::agent::PermissionMode;
use crate::provider::{AgentConfig, AgentOutput, DebugMessage, DebugToolCall};
use crate::utils::estimate_tokens;

/// Default timeout for a single Claude CLI invocation (5 minutes).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Return the context window size for a known Claude model identifier.
///
/// Returns `200_000` for all standard models, `1_000_000` for `[1m]` variants,
/// and `200_000` as a safe default for unrecognised identifiers.
pub fn context_window_for_model(model: &str) -> usize {
    if model.ends_with("[1m]") {
        1_000_000
    } else {
        200_000
    }
}

/// Validate that the prompt fits within the Claude model's context window.
///
/// # Errors
///
/// Returns [`AgentError::PromptTooLarge`] if the estimated token count exceeds
/// the model's context window.
pub fn validate_prompt_size(config: &AgentConfig) -> Result<(), AgentError> {
    let total_chars = config.prompt.len() + config.system_prompt.as_ref().map_or(0, |s| s.len());
    let estimated_tokens = estimate_tokens(total_chars);
    let model_limit = context_window_for_model(&config.model);
    if estimated_tokens > model_limit {
        return Err(AgentError::PromptTooLarge {
            chars: total_chars,
            estimated_tokens,
            model_limit,
        });
    }
    Ok(())
}

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
    collect_vars_to_remove(env::vars().map(|(k, _)| k))
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
    let output_format = if config.verbose {
        "stream-json"
    } else {
        "json"
    };

    let mut args: Vec<String> = vec![
        "-p".to_string(),
        config.prompt.clone(),
        "--output-format".to_string(),
        output_format.to_string(),
    ];

    // Claude CLI requires --verbose when using --output-format=stream-json with -p
    if config.verbose {
        args.push("--verbose".to_string());
    }

    push_opt(&mut args, "--system-prompt", &config.system_prompt);
    push_flag(&mut args, "--model", &config.model);
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
            if env::var("IRONFLOW_ALLOW_BYPASS").as_deref() != Ok("1") {
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
            debug_messages: Vec::new(),
        })?;

    let value = if config.json_schema.is_some() {
        extract_structured_value(&parsed).ok_or_else(|| {
            warn!(
                subtype = ?parsed.subtype,
                result_is_null = parsed.result.as_ref().is_none_or(|v| v.is_null()),
                structured_output_is_null = parsed.structured_output.as_ref().is_none_or(|v| v.is_null()),
                has_tools = !config.allowed_tools.is_empty(),
                "structured_output extraction failed, dumping response fields for diagnosis"
            );
            if let Some(ref result) = parsed.result {
                let preview = result.to_string();
                let truncated = &preview[..preview.len().min(2000)];
                warn!(result_preview = truncated, "result field content (truncated to 2000 chars)");
            }
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
                debug_messages: Vec::new(),
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
        debug_messages: None,
    })
}

/// Parse `stream-json` output from the `claude` CLI into an [`AgentOutput`]
/// with conversation trace in [`AgentOutput::debug_messages`].
///
/// The `stream-json` format emits one JSON object per line. Lines with
/// `"type":"assistant"` carry conversation content (text and tool calls).
/// The final `"type":"result"` line carries the same payload as the `json`
/// format and is used to populate the standard output fields.
///
/// # Errors
///
/// Returns [`AgentError::SchemaValidation`] if the result line is missing
/// or cannot be parsed.
pub fn parse_stream_response(
    stdout: &str,
    config: &AgentConfig,
    fallback_duration_ms: u64,
) -> Result<AgentOutput, AgentError> {
    let mut debug_messages: Vec<DebugMessage> = Vec::new();
    let mut result_line: Option<&str> = None;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        match parsed.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => {
                let message = parsed.get("message");
                let content = message
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array());
                let stop_reason = message
                    .and_then(|m| m.get("stop_reason"))
                    .and_then(|s| s.as_str())
                    .map(|s| s.to_string());

                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<DebugToolCall> = Vec::new();

                if let Some(blocks) = content {
                    for block in blocks {
                        match block.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                    text_parts.push(t.to_string());
                                }
                            }
                            Some("tool_use") => {
                                let name = block
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let input = block.get("input").cloned().unwrap_or(Value::Null);
                                tool_calls.push(DebugToolCall { name, input });
                            }
                            _ => {}
                        }
                    }
                }

                let text = if text_parts.is_empty() {
                    None
                } else {
                    Some(text_parts.join("\n"))
                };

                debug_messages.push(DebugMessage {
                    text,
                    tool_calls,
                    stop_reason,
                });
            }
            Some("result") => {
                result_line = Some(trimmed);
            }
            _ => {}
        }
    }

    let result_str = match result_line {
        Some(line) => line,
        None => {
            return Err(AgentError::SchemaValidation {
                expected: "stream-json result line".to_string(),
                got: "no result line found in stream output".to_string(),
                debug_messages,
            });
        }
    };

    match parse_response(result_str, config, fallback_duration_ms) {
        Ok(mut output) => {
            output.debug_messages = Some(debug_messages);
            Ok(output)
        }
        Err(AgentError::SchemaValidation { expected, got, .. }) => {
            Err(AgentError::SchemaValidation {
                expected,
                got,
                debug_messages,
            })
        }
        Err(other) => Err(other),
    }
}

/// Parse CLI output, dispatching to the correct parser based on verbose mode.
///
/// When [`AgentConfig::verbose`] is `true`, uses [`parse_stream_response`] to
/// extract the full conversation trace. Otherwise uses [`parse_response`] for
/// the standard single-JSON output.
///
/// # Errors
///
/// Returns [`AgentError`] if parsing fails (see individual parsers).
pub fn parse_output(
    stdout: &str,
    config: &AgentConfig,
    fallback_duration_ms: u64,
) -> Result<AgentOutput, AgentError> {
    if config.verbose {
        parse_stream_response(stdout, config, fallback_duration_ms)
    } else {
        parse_response(stdout, config, fallback_duration_ms)
    }
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

    #[test]
    fn build_args_verbose_uses_stream_json_and_verbose_flag() {
        let mut config = AgentConfig::new("hello");
        config.verbose = true;
        let args = build_args(&config).unwrap();
        assert_eq!(args[2], "--output-format");
        assert_eq!(args[3], "stream-json");
        assert!(
            args.contains(&"--verbose".to_string()),
            "stream-json with -p requires --verbose flag, got: {args:?}"
        );
    }

    #[test]
    fn build_args_non_verbose_uses_json() {
        let config = AgentConfig::new("hello");
        let args = build_args(&config).unwrap();
        assert_eq!(args[3], "json");
        assert!(
            !args.contains(&"--verbose".to_string()),
            "--verbose should not be present when verbose is false"
        );
    }

    #[test]
    fn parse_stream_response_extracts_messages_and_result() {
        let stream = [
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Let me read that file."},{"type":"tool_use","id":"tu_1","name":"Read","input":{"file_path":"/tmp/test.rs"}}],"stop_reason":"tool_use"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"Done."}],"stop_reason":"end_turn"}}"#,
            r#"{"type":"result","session_id":"s1","result":"Done.","usage":{"input_tokens":100,"output_tokens":50},"total_cost_usd":0.02,"duration_ms":500}"#,
        ]
        .join("\n");

        let config = AgentConfig::new("test");
        let output = parse_stream_response(&stream, &config, 999).unwrap();

        assert_eq!(output.value, Value::String("Done.".to_string()));
        assert_eq!(output.session_id, Some("s1".to_string()));
        assert_eq!(output.duration_ms, 500);

        let messages = output.debug_messages.unwrap();
        assert_eq!(messages.len(), 2);

        assert_eq!(messages[0].text.as_deref(), Some("Let me read that file."));
        assert_eq!(messages[0].tool_calls.len(), 1);
        assert_eq!(messages[0].tool_calls[0].name, "Read");
        assert_eq!(messages[0].stop_reason.as_deref(), Some("tool_use"));

        assert_eq!(messages[1].text.as_deref(), Some("Done."));
        assert!(messages[1].tool_calls.is_empty());
        assert_eq!(messages[1].stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn parse_stream_response_no_result_line_errors() {
        let stream = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi"}],"stop_reason":"end_turn"}}"#;
        let config = AgentConfig::new("test");
        let result = parse_stream_response(stream, &config, 0);
        assert!(result.is_err());
    }

    #[test]
    fn parse_stream_response_empty_stream_errors() {
        let config = AgentConfig::new("test");
        let result = parse_stream_response("", &config, 0);
        assert!(result.is_err());
    }

    #[test]
    fn parse_stream_response_skips_invalid_lines() {
        let stream = [
            "not json",
            "",
            r#"{"type":"result","result":"ok","duration_ms":100}"#,
        ]
        .join("\n");

        let config = AgentConfig::new("test");
        let output = parse_stream_response(&stream, &config, 999).unwrap();
        assert_eq!(output.value, Value::String("ok".to_string()));
        let messages = output.debug_messages.unwrap();
        assert!(messages.is_empty());
    }

    #[test]
    fn parse_stream_response_multiple_tool_calls_in_one_turn() {
        let stream = [
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Grep","input":{"pattern":"foo"}},{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/tmp/bar"}}],"stop_reason":"tool_use"}}"#,
            r#"{"type":"result","result":"done","duration_ms":200}"#,
        ]
        .join("\n");

        let config = AgentConfig::new("test");
        let output = parse_stream_response(&stream, &config, 0).unwrap();
        let messages = output.debug_messages.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].tool_calls.len(), 2);
        assert_eq!(messages[0].tool_calls[0].name, "Grep");
        assert_eq!(messages[0].tool_calls[1].name, "Read");
        assert!(messages[0].text.is_none());
    }

    #[test]
    fn debug_message_display_format() {
        let msg = DebugMessage {
            text: Some("Analyzing...".to_string()),
            tool_calls: vec![DebugToolCall {
                name: "Read".to_string(),
                input: json!({"file_path": "/tmp/test.rs"}),
            }],
            stop_reason: Some("tool_use".to_string()),
        };
        let display = format!("{msg}");
        assert!(display.contains("[assistant] Analyzing..."));
        assert!(display.contains("[tool_use] Read"));
    }

    #[test]
    fn build_args_includes_both_tools_and_json_schema() {
        use std::marker::PhantomData;

        use crate::operations::agent::PermissionMode;

        // Use direct field access to bypass typestate (testing CLI arg construction,
        // not the builder API -- this combination triggers a Claude CLI bug).
        let config = AgentConfig {
            prompt: "test prompt".to_string(),
            model: "sonnet".to_string(),
            json_schema: Some(
                r#"{"type":"object","properties":{"items":{"type":"array"}}}"#.to_string(),
            ),
            allowed_tools: vec!["WebSearch".to_string(), "WebFetch".to_string()],
            max_turns: Some(5),
            permission_mode: PermissionMode::Default,
            system_prompt: None,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            resume_session_id: None,
            verbose: false,
            _marker: PhantomData,
        };

        let args = build_args(&config).unwrap();

        assert!(args.contains(&"--allowedTools".to_string()));
        assert!(args.contains(&"WebSearch,WebFetch".to_string()));
        assert!(args.contains(&"--json-schema".to_string()));
        assert!(
            args.contains(
                &r#"{"type":"object","properties":{"items":{"type":"array"}}}"#.to_string()
            )
        );
        assert!(args.contains(&"--output-format".to_string()));
        assert!(args.contains(&"json".to_string()));
    }

    #[test]
    fn stream_response_preserves_debug_messages_on_schema_validation_error() {
        let assistant_line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Searching..."},{"type":"tool_use","name":"WebSearch","input":{"query":"AI news"}}],"stop_reason":"tool_use"}}"#;
        let result_line = r#"{"type":"result","session_id":"s1","subtype":"success","result":"text response","usage":{"input_tokens":100,"output_tokens":50},"total_cost_usd":0.01,"duration_ms":500}"#;

        let stdout = format!("{assistant_line}\n{result_line}");

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();
        let config = config.verbose(true);

        let err = parse_stream_response(&stdout, &config, 500).unwrap_err();

        match err {
            AgentError::SchemaValidation { debug_messages, .. } => {
                assert_eq!(debug_messages.len(), 1);
                assert_eq!(debug_messages[0].text.as_deref(), Some("Searching..."));
                assert_eq!(debug_messages[0].tool_calls.len(), 1);
                assert_eq!(debug_messages[0].tool_calls[0].name, "WebSearch");
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_schema_validation_error_has_empty_debug_messages() {
        let stdout = r#"{"session_id":"s1","subtype":"success","result":"plain text","usage":{"input_tokens":10,"output_tokens":5},"total_cost_usd":0.01,"duration_ms":100}"#;

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();

        let err = parse_response(stdout, &config, 100).unwrap_err();

        match err {
            AgentError::SchemaValidation { debug_messages, .. } => {
                assert!(debug_messages.is_empty());
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }
}
