//! Shared utilities for all Claude Code transport providers.
//!
//! This module contains command-line argument building, JSON response parsing,
//! and structured-output extraction logic shared across local, SSH, Docker,
//! and Kubernetes transports.

use std::env;
#[cfg(any(
    feature = "transport-ssh",
    feature = "transport-docker",
    feature = "transport-k8s"
))]
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Map, Value};
#[cfg(any(
    feature = "transport-ssh",
    feature = "transport-docker",
    feature = "transport-k8s"
))]
use tracing::debug;
use tracing::{trace, warn};

use crate::error::{AgentError, PartialUsage};
use crate::operations::agent::PermissionMode;
#[cfg(any(
    feature = "transport-ssh",
    feature = "transport-docker",
    feature = "transport-k8s"
))]
use crate::provider::LogSink;
use crate::provider::{AgentConfig, AgentOutput, DebugMessage, DebugToolCall, DebugToolResult};
use crate::schema_transform::transform_schema;
use crate::utils::estimate_tokens;

/// Default timeout for a single Claude CLI invocation (5 minutes).
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// Byte threshold above which the prompt is piped via stdin instead of
/// passed as a `-p` CLI argument. This avoids the OS `ARG_MAX` limit
/// (typically 256 KB on macOS, 2 MB on Linux) when reviewing large diffs.
pub const PROMPT_STDIN_THRESHOLD: usize = 100_000;

/// Maximum byte length for raw response data included in error diagnostics.
const RAW_RESPONSE_MAX_LEN: usize = 4000;

/// Approximate byte limit for stdout fallback in error diagnostics.
const RAW_RESPONSE_FALLBACK_MAX_LEN: usize = 2000;

/// Maximum byte length for error detail in `ProcessFailed` errors.
///
/// 50 KB is enough for diagnostic context while staying well under typical
/// DB column and API payload limits.
const ERROR_DETAIL_MAX_LEN: usize = 50_000;

/// Truncate a string to at most `max_len` bytes on a char boundary.
fn truncate_to(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        return s.to_string();
    }
    let end = s.floor_char_boundary(max_len);
    format!("{}...(truncated)", &s[..end])
}

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

/// Return a clone of `config` with `verbose` forced to `true` when streaming
/// is active but verbose is off. Returns `None` when no override is needed.
#[cfg(any(
    feature = "transport-ssh",
    feature = "transport-docker",
    feature = "transport-k8s"
))]
pub(super) fn force_verbose_for_streaming(
    config: &AgentConfig,
    streaming: bool,
) -> Option<AgentConfig> {
    if streaming && !config.verbose {
        debug!("forcing verbose=true for log streaming (stream-json output required)");
        Some(config.clone().verbose(true))
    } else {
        None
    }
}

/// Forward raw output data to a [`LogSink`], splitting by line.
#[cfg(any(
    feature = "transport-ssh",
    feature = "transport-docker",
    feature = "transport-k8s"
))]
pub(super) fn stream_lines(data: &[u8], stream: &str, sink: Option<&Arc<dyn LogSink>>) {
    if let Some(sink) = sink {
        let text = String::from_utf8_lossy(data);
        for line in text.lines() {
            sink.log(stream, line);
        }
    }
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
    if !config.disallowed_tools.is_empty() {
        push_flag(
            &mut args,
            "--disallowedTools",
            &config.disallowed_tools.join(","),
        );
    }
    push_opt(&mut args, "--max-turns", &config.max_turns);
    push_opt(&mut args, "--max-budget-usd", &config.max_budget_usd);
    push_opt(&mut args, "--mcp-config", &config.mcp_config);
    if config.strict_mcp_config {
        args.push("--strict-mcp-config".to_string());
    }
    if config.bare {
        args.push("--bare".to_string());
    }

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

    let transformed_schema = config.json_schema.as_ref().map(|s| transform_schema(s));
    push_opt(&mut args, "--json-schema", &transformed_schema);

    if let Some(ref session_id) = config.resume_session_id {
        args.push("--resume".to_string());
        args.push(session_id.clone());
    }

    Ok(args)
}

/// CLI arguments and an optional prompt payload for stdin piping.
///
/// When the prompt exceeds [`PROMPT_STDIN_THRESHOLD`], it is excluded from
/// the argument list and placed in [`stdin_prompt`](Self::stdin_prompt)
/// instead. Each transport provider is responsible for piping it to the
/// `claude` process's stdin.
#[derive(Debug)]
pub struct BuiltCommand {
    /// Arguments to pass after the `claude` binary name.
    pub args: Vec<String>,
    /// Prompt text that must be written to stdin. `None` when the prompt
    /// is small enough to fit in the CLI arguments.
    pub stdin_prompt: Option<String>,
}

/// Build CLI arguments, splitting the prompt to stdin when it is too large
/// for the OS argument limit.
///
/// Providers should use this instead of [`build_args`] to handle large
/// prompts gracefully.
///
/// # Errors
///
/// Returns [`AgentError::ProcessFailed`] if `BypassPermissions` is requested
/// without the `IRONFLOW_ALLOW_BYPASS=1` environment variable.
pub fn build_command(config: &AgentConfig) -> Result<BuiltCommand, AgentError> {
    let prompt_via_stdin = config.prompt.len() > PROMPT_STDIN_THRESHOLD;

    let output_format = if config.verbose {
        "stream-json"
    } else {
        "json"
    };

    let mut args: Vec<String> = Vec::with_capacity(16);
    if !prompt_via_stdin {
        args.push("-p".to_string());
        args.push(config.prompt.clone());
    }
    args.push("--output-format".to_string());
    args.push(output_format.to_string());

    if config.verbose {
        args.push("--verbose".to_string());
    }

    push_opt(&mut args, "--system-prompt", &config.system_prompt);
    push_flag(&mut args, "--model", &config.model);
    if !config.allowed_tools.is_empty() {
        push_flag(&mut args, "--allowedTools", &config.allowed_tools.join(","));
    }
    if !config.disallowed_tools.is_empty() {
        push_flag(
            &mut args,
            "--disallowedTools",
            &config.disallowed_tools.join(","),
        );
    }
    push_opt(&mut args, "--max-turns", &config.max_turns);
    push_opt(&mut args, "--max-budget-usd", &config.max_budget_usd);
    push_opt(&mut args, "--mcp-config", &config.mcp_config);
    if config.strict_mcp_config {
        args.push("--strict-mcp-config".to_string());
    }
    if config.bare {
        args.push("--bare".to_string());
    }

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

    let transformed_schema = config.json_schema.as_ref().map(|s| transform_schema(s));
    push_opt(&mut args, "--json-schema", &transformed_schema);

    if let Some(ref session_id) = config.resume_session_id {
        args.push("--resume".to_string());
        args.push(session_id.clone());
    }

    Ok(BuiltCommand {
        args,
        stdin_prompt: if prompt_via_stdin {
            Some(config.prompt.clone())
        } else {
            None
        },
    })
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
///
/// # Why the fallbacks exist
///
/// Claude CLI has several known bugs around structured output that make
/// the `structured_output` field unreliable:
///
/// - When tools are used alongside `--json-schema`, `structured_output`
///   is always `null` (the result lands in `result` as markdown text).
///   See <https://github.com/anthropics/claude-code/issues/18536>.
///   This case is blocked at compile time by the typestate, but defensive
///   fallbacks remain for forward compatibility.
/// - The CLI does not validate output against the schema; it may return
///   malformed or non-conforming JSON.
///   See <https://github.com/anthropics/claude-code/issues/9058>.
/// - Wrapper objects with a single array field may be flattened to a bare
///   array non-deterministically.
///   See <https://github.com/anthropics/claude-agent-sdk-python/issues/502>
///   and <https://github.com/anthropics/claude-agent-sdk-python/issues/374>.
///
/// Because of these issues, we try multiple extraction strategies in order:
/// 1. `structured_output` field (when non-null)
/// 2. Direct JSON parse of `result`
/// 3. JSON code fence extraction from `result`
/// 4. First `{...}` brace extraction from `result`
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

/// Extract the raw response text from a parsed CLI response for error diagnostics.
///
/// Prefers the `result` field (stringified and truncated); falls back to
/// raw stdout when `result` is null.
fn extract_raw_response_text(parsed: &ClaudeJsonOutput, stdout: &str) -> Option<String> {
    if let Some(ref result) = parsed.result
        && !result.is_null()
    {
        let text = match result.as_str() {
            Some(s) => s.to_string(),
            None => result.to_string(),
        };
        return Some(truncate_to(&text, RAW_RESPONSE_MAX_LEN));
    }
    if !stdout.is_empty() {
        return Some(truncate_to(stdout, RAW_RESPONSE_FALLBACK_MAX_LEN));
    }
    None
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
            partial_usage: Box::default(),
            raw_response: Some(truncate_to(stdout, RAW_RESPONSE_MAX_LEN)),
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
            let usage = Box::new(PartialUsage {
                cost_usd: parsed.total_cost_usd,
                duration_ms: parsed.duration_ms,
                input_tokens: parsed.usage.as_ref().map(|u| u.total_input_tokens()),
                output_tokens: parsed.usage.as_ref().map(|u| u.total_output_tokens()),
            });

            // The budget running out is not a schema problem: retrying spends
            // more money and cannot succeed. Report it as its own error so the
            // retry layers can refuse to replay it.
            if parsed.subtype.as_deref() == Some("error_max_budget_usd") {
                return AgentError::BudgetExceeded {
                    spent_usd: parsed.total_cost_usd.unwrap_or(0.0),
                    limit_usd: config.max_budget_usd.unwrap_or(0.0),
                    debug_messages: Vec::new(),
                    partial_usage: usage,
                };
            }

            let hint = match parsed.subtype.as_deref() {
                Some("error_max_turns") => {
                    " (max turns reached before structured output was generated - use max_turns >= 2 with structured output)"
                }
                Some(sub) => {
                    warn!(subtype = sub, "claude returned no structured_output");
                    ""
                }
                None => "",
            };
            let raw_response = extract_raw_response_text(&parsed, stdout);

            AgentError::SchemaValidation {
                expected: "structured_output field".to_string(),
                got: format!("null{hint}"),
                debug_messages: Vec::new(),
                partial_usage: usage,
                raw_response,
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
    // True when the next `assistant` line should merge into the last pushed
    // `DebugMessage`. A Claude CLI turn can span several `assistant` lines
    // (one per content_block: thinking, tool_use, text). We aggregate them
    // until a `user` (tool_result) line or a terminal `stop_reason` arrives.
    let mut assistant_turn_open = false;

    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let parsed: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let line_type = parsed
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("<missing>");
        let content_kinds: Vec<&str> = parsed
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_array())
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b.get("type").and_then(|t| t.as_str()))
                    .collect()
            })
            .unwrap_or_default();
        trace!(
            target: "ironflow_core::stream",
            line_type,
            content_kinds = ?content_kinds,
            raw_len = trimmed.len(),
            "stream-json line"
        );

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

                let usage = message.and_then(|m| m.get("usage"));
                let input_tokens = usage
                    .and_then(|u| u.get("input_tokens"))
                    .and_then(|v| v.as_u64());
                let output_tokens = usage
                    .and_then(|u| u.get("output_tokens"))
                    .and_then(|v| v.as_u64());

                let mut text_parts: Vec<String> = Vec::new();
                let mut thinking_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<DebugToolCall> = Vec::new();
                let mut thinking_redacted = false;

                if let Some(blocks) = content {
                    for block in blocks {
                        match block.get("type").and_then(|t| t.as_str()) {
                            Some("text") => {
                                if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                                    text_parts.push(t.to_string());
                                }
                            }
                            Some("thinking") => {
                                // Try the canonical field first, fall back to
                                // `text` which some Claude CLI versions use.
                                let text_value = block
                                    .get("thinking")
                                    .and_then(|t| t.as_str())
                                    .or_else(|| block.get("text").and_then(|t| t.as_str()));
                                if let Some(t) = text_value
                                    && !t.is_empty()
                                {
                                    thinking_parts.push(t.to_string());
                                } else {
                                    // Opus 4.7 adaptive thinking and
                                    // `display: "omitted"` yield signature-only
                                    // thinking blocks. Flag them so the UI can
                                    // still surface that reasoning happened.
                                    thinking_redacted = true;
                                }
                            }
                            Some("tool_use") => {
                                let id = block
                                    .get("id")
                                    .and_then(|n| n.as_str())
                                    .map(|s| s.to_string());
                                let name = block
                                    .get("name")
                                    .and_then(|n| n.as_str())
                                    .unwrap_or("unknown")
                                    .to_string();
                                let input = block.get("input").cloned().unwrap_or(Value::Null);
                                tool_calls.push(DebugToolCall { id, name, input });
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
                let thinking = if thinking_parts.is_empty() {
                    None
                } else {
                    Some(thinking_parts.join("\n"))
                };

                let stop_is_terminal = stop_reason.is_some();

                if assistant_turn_open && let Some(last) = debug_messages.last_mut() {
                    // Merge into the turn still being built.
                    if let Some(t) = text {
                        last.text = Some(match last.text.take() {
                            Some(existing) if !existing.is_empty() => format!("{existing}\n{t}"),
                            _ => t,
                        });
                    }
                    if let Some(t) = thinking {
                        last.thinking = Some(match last.thinking.take() {
                            Some(existing) if !existing.is_empty() => format!("{existing}\n{t}"),
                            _ => t,
                        });
                    }
                    last.thinking_redacted = last.thinking_redacted || thinking_redacted;
                    last.tool_calls.extend(tool_calls);
                    if stop_is_terminal {
                        last.stop_reason = stop_reason;
                    }
                    if let Some(v) = input_tokens {
                        last.input_tokens = Some(last.input_tokens.unwrap_or(0) + v);
                    }
                    if let Some(v) = output_tokens {
                        last.output_tokens = Some(last.output_tokens.unwrap_or(0) + v);
                    }
                } else {
                    debug_messages.push(DebugMessage {
                        text,
                        thinking,
                        thinking_redacted,
                        tool_calls,
                        tool_results: Vec::new(),
                        stop_reason,
                        input_tokens,
                        output_tokens,
                    });
                }

                // Keep aggregating unless the CLI signalled the end of the turn.
                assistant_turn_open = !stop_is_terminal;
            }
            Some("user") => {
                // A tool_result always closes the current assistant turn.
                assistant_turn_open = false;
                // Tool results come as user messages whose content is an array
                // of `tool_result` blocks. Attach them to the most recent
                // assistant turn that emitted matching tool_use entries so
                // the timeline stays compact.
                let content = parsed
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(|c| c.as_array());

                if let Some(blocks) = content {
                    for block in blocks {
                        if block.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                            let tool_use_id = block
                                .get("tool_use_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let content_value =
                                block.get("content").cloned().unwrap_or(Value::Null);
                            let is_error = block
                                .get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);

                            let result = DebugToolResult {
                                tool_use_id: tool_use_id.clone(),
                                content: content_value,
                                is_error,
                            };

                            // Attach to the turn whose tool_calls include this id.
                            let target = tool_use_id.as_deref().and_then(|id| {
                                debug_messages.iter_mut().rev().find(|m| {
                                    m.tool_calls.iter().any(|tc| tc.id.as_deref() == Some(id))
                                })
                            });

                            if let Some(msg) = target {
                                msg.tool_results.push(result);
                            } else if let Some(last) = debug_messages.last_mut() {
                                last.tool_results.push(result);
                            }
                        }
                    }
                }
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
                partial_usage: Box::default(),
                raw_response: if stdout.is_empty() {
                    None
                } else {
                    Some(truncate_to(stdout, RAW_RESPONSE_FALLBACK_MAX_LEN))
                },
            });
        }
    };

    match parse_response(result_str, config, fallback_duration_ms) {
        Ok(mut output) => {
            output.debug_messages = Some(debug_messages);
            Ok(output)
        }
        Err(AgentError::SchemaValidation {
            expected,
            got,
            partial_usage,
            raw_response,
            ..
        }) => Err(AgentError::SchemaValidation {
            expected,
            got,
            debug_messages,
            partial_usage,
            raw_response,
        }),
        Err(AgentError::BudgetExceeded {
            spent_usd,
            limit_usd,
            partial_usage,
            ..
        }) => Err(AgentError::BudgetExceeded {
            spent_usd,
            limit_usd,
            debug_messages,
            partial_usage,
        }),
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

/// Check whether a [`SchemaValidation`](AgentError::SchemaValidation) error
/// contains real usage data from the CLI (cost or duration present).
fn has_usage_data(err: &AgentError) -> bool {
    if let AgentError::SchemaValidation { partial_usage, .. } = err {
        return partial_usage.cost_usd.is_some() || partial_usage.duration_ms.is_some();
    }
    false
}

/// Handle a non-zero exit code from the Claude CLI.
///
/// The CLI exits with code 1 for budget/turn limit errors but still writes
/// valid JSON (with usage data) to stdout or stderr. This function tries to
/// parse that output so cost, duration, and tokens are preserved in the error.
///
/// Returns `Ok` if the JSON is a valid successful response (rare but possible),
/// `Err(BudgetExceeded)` when the CLI reported `error_max_budget_usd`,
/// `Err(SchemaValidation)` with partial usage when structured output was
/// requested but missing, or `Err(ProcessFailed)` as a fallback.
pub fn handle_nonzero_exit(
    exit_code: i32,
    stdout: &str,
    stderr: &str,
    config: &AgentConfig,
    duration_ms: u64,
    log_prefix: &str,
) -> Result<AgentOutput, AgentError> {
    let json_source = if stdout.is_empty() { stderr } else { stdout };

    if !json_source.is_empty() {
        match parse_output(json_source, config, duration_ms) {
            ok @ Ok(_) => return ok,
            // Always carries the usage the CLI reported before stopping.
            Err(err @ AgentError::BudgetExceeded { .. }) => return Err(err),
            Err(err @ AgentError::SchemaValidation { .. }) => {
                if has_usage_data(&err) {
                    return Err(err);
                }
                // No usage data means the JSON wasn't a real CLI response
                // (e.g. a parse error). Fall through to ProcessFailed.
            }
            Err(_) => {} // not parseable, fall through
        }
    }

    let error_detail = if stdout.is_empty() {
        if stderr.is_empty() {
            "(no output captured)".to_string()
        } else {
            truncate_to(stderr, ERROR_DETAIL_MAX_LEN)
        }
    } else {
        truncate_to(stdout, ERROR_DETAIL_MAX_LEN)
    };

    tracing::error!(
        exit_code,
        error_detail_len = error_detail.len(),
        "{log_prefix} claude process failed"
    );

    Err(AgentError::ProcessFailed {
        exit_code,
        stderr: error_detail,
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
    fn build_args_strict_mcp_config_flag_absent_by_default() {
        let config = AgentConfig::new("hello");
        let args = build_args(&config).unwrap();
        assert!(
            !args.contains(&"--strict-mcp-config".to_string()),
            "--strict-mcp-config must not appear unless opted-in, got: {args:?}"
        );
    }

    #[test]
    fn build_args_strict_mcp_config_flag_pushed_when_enabled() {
        let config = AgentConfig::new("hello").strict_mcp_config(true);
        let args = build_args(&config).unwrap();
        assert!(
            args.contains(&"--strict-mcp-config".to_string()),
            "--strict-mcp-config must be pushed when strict_mcp_config is true, got: {args:?}"
        );
    }

    #[test]
    fn build_args_disallowed_tools_flag_absent_when_empty() {
        let config = AgentConfig::new("hello");
        let args = build_args(&config).unwrap();
        assert!(
            !args.contains(&"--disallowedTools".to_string()),
            "--disallowedTools must not appear when list is empty, got: {args:?}"
        );
    }

    #[test]
    fn build_args_disallowed_tools_flag_joined_with_commas() {
        let config = AgentConfig::new("hello").disallowed_tools(["Write", "Edit", "Bash"]);
        let args = build_args(&config).unwrap();

        let pos = args
            .iter()
            .position(|a| a == "--disallowedTools")
            .expect("--disallowedTools missing");
        assert_eq!(args[pos + 1], "Write,Edit,Bash");
    }

    #[test]
    fn build_args_disallowed_tools_combined_with_allowed_tools() {
        let config: AgentConfig = AgentConfig::new("hello")
            .allow_tool("Read")
            .allow_tool("Grep")
            .into();
        let config = config.disallowed_tools(["Write", "Edit"]);
        let args = build_args(&config).unwrap();

        let allowed_pos = args
            .iter()
            .position(|a| a == "--allowedTools")
            .expect("--allowedTools missing");
        assert_eq!(args[allowed_pos + 1], "Read,Grep");

        let disallowed_pos = args
            .iter()
            .position(|a| a == "--disallowedTools")
            .expect("--disallowedTools missing");
        assert_eq!(args[disallowed_pos + 1], "Write,Edit");
    }

    #[test]
    fn build_args_bare_flag_absent_by_default() {
        let config = AgentConfig::new("hello");
        let args = build_args(&config).unwrap();
        assert!(
            !args.contains(&"--bare".to_string()),
            "--bare must not appear unless opted-in, got: {args:?}"
        );
    }

    #[test]
    fn build_args_bare_flag_pushed_when_enabled() {
        let config = AgentConfig::new("hello").bare(true);
        let args = build_args(&config).unwrap();
        assert!(
            args.contains(&"--bare".to_string()),
            "--bare must be pushed when bare is true, got: {args:?}"
        );
    }

    #[test]
    fn build_args_strict_mcp_config_with_mcp_config_includes_both() {
        let config = AgentConfig::new("hello")
            .mcp_config(r#"{"mcpServers":{}}"#)
            .strict_mcp_config(true);
        let args = build_args(&config).unwrap();

        let mcp_pos = args
            .iter()
            .position(|a| a == "--mcp-config")
            .expect("--mcp-config missing");
        assert_eq!(args[mcp_pos + 1], r#"{"mcpServers":{}}"#);
        assert!(
            args.contains(&"--strict-mcp-config".to_string()),
            "--strict-mcp-config missing when both flags requested"
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
            thinking: None,
            thinking_redacted: false,
            tool_calls: vec![DebugToolCall {
                id: Some("tu_1".to_string()),
                name: "Read".to_string(),
                input: json!({"file_path": "/tmp/test.rs"}),
            }],
            tool_results: Vec::new(),
            stop_reason: Some("tool_use".to_string()),
            input_tokens: None,
            output_tokens: None,
        };
        let display = format!("{msg}");
        assert!(display.contains("[assistant] Analyzing..."));
        assert!(display.contains("[tool_use] Read"));
    }

    #[test]
    fn parse_stream_response_flags_redacted_thinking() {
        let stream = [
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"","signature":"sig_abc"}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"ls"}}],"stop_reason":"tool_use"}}"#,
            r#"{"type":"result","result":"","duration_ms":100}"#,
        ]
        .join("\n");
        let config = AgentConfig::new("test");
        let output = parse_stream_response(&stream, &config, 0).unwrap();
        let messages = output.debug_messages.unwrap();
        assert_eq!(messages.len(), 1);
        assert!(messages[0].thinking_redacted);
        assert!(messages[0].thinking.is_none());
        assert_eq!(messages[0].tool_calls.len(), 1);
    }

    #[test]
    fn parse_stream_response_extracts_thinking_block() {
        let stream = [
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"Let me reason about this step by step."},{"type":"text","text":"Answer: 42"}],"stop_reason":"end_turn","usage":{"input_tokens":120,"output_tokens":30}}}"#,
            r#"{"type":"result","result":"Answer: 42","duration_ms":250}"#,
        ]
        .join("\n");
        let config = AgentConfig::new("test");
        let output = parse_stream_response(&stream, &config, 0).unwrap();
        let messages = output.debug_messages.unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(
            messages[0].thinking.as_deref(),
            Some("Let me reason about this step by step.")
        );
        assert_eq!(messages[0].text.as_deref(), Some("Answer: 42"));
        assert_eq!(messages[0].input_tokens, Some(120));
        assert_eq!(messages[0].output_tokens, Some(30));
    }

    #[test]
    fn parse_stream_response_attaches_tool_results_to_matching_turn() {
        let stream = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_1","name":"Read","input":{"file_path":"/tmp/a"}}],"stop_reason":"tool_use"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu_1","content":"file contents here","is_error":false}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}],"stop_reason":"end_turn"}}"#,
            r#"{"type":"result","result":"Done.","duration_ms":400}"#,
        ]
        .join("\n");
        let config = AgentConfig::new("test");
        let output = parse_stream_response(&stream, &config, 0).unwrap();
        let messages = output.debug_messages.unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].tool_calls.len(), 1);
        assert_eq!(messages[0].tool_calls[0].id.as_deref(), Some("tu_1"));
        assert_eq!(messages[0].tool_results.len(), 1);
        assert_eq!(
            messages[0].tool_results[0].tool_use_id.as_deref(),
            Some("tu_1")
        );
        assert!(!messages[0].tool_results[0].is_error);
        assert!(messages[1].tool_results.is_empty());
    }

    #[test]
    fn parse_stream_response_merges_consecutive_assistant_content_blocks() {
        // The Claude CLI emits one `assistant` line per content block:
        // thinking first (no stop_reason), then tool_use (stop_reason=tool_use).
        // Both belong to the same logical turn and must be collapsed into one
        // DebugMessage.
        let stream = [
            r#"{"type":"assistant","message":{"content":[{"type":"thinking","thinking":"I should list files first."}],"usage":{"input_tokens":6,"output_tokens":0}}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_1","name":"Bash","input":{"command":"ls"}}],"stop_reason":"tool_use","usage":{"input_tokens":1,"output_tokens":65}}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu_1","content":"README.md","is_error":false}]}}"#,
            r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Done."}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":1}}}"#,
            r#"{"type":"result","result":"Done.","duration_ms":500}"#,
        ]
        .join("\n");
        let config = AgentConfig::new("test");
        let output = parse_stream_response(&stream, &config, 0).unwrap();
        let messages = output.debug_messages.unwrap();

        assert_eq!(
            messages.len(),
            2,
            "expected 2 logical turns, got {messages:?}"
        );

        // Turn 1: thinking + tool_use merged, tool_result attached.
        assert_eq!(
            messages[0].thinking.as_deref(),
            Some("I should list files first.")
        );
        assert_eq!(messages[0].tool_calls.len(), 1);
        assert_eq!(messages[0].tool_calls[0].id.as_deref(), Some("tu_1"));
        assert_eq!(messages[0].tool_results.len(), 1);
        assert_eq!(messages[0].stop_reason.as_deref(), Some("tool_use"));
        assert_eq!(messages[0].input_tokens, Some(7));
        assert_eq!(messages[0].output_tokens, Some(65));

        // Turn 2: the final assistant text.
        assert_eq!(messages[1].text.as_deref(), Some("Done."));
        assert_eq!(messages[1].stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn parse_stream_response_marks_tool_result_error() {
        let stream = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"tu_x","name":"Bash","input":{"command":"boom"}}],"stop_reason":"tool_use"}}"#,
            r#"{"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tu_x","content":"command failed","is_error":true}]}}"#,
            r#"{"type":"result","result":"error","duration_ms":100}"#,
        ]
        .join("\n");
        let config = AgentConfig::new("test");
        let output = parse_stream_response(&stream, &config, 0).unwrap();
        let messages = output.debug_messages.unwrap();
        assert_eq!(messages[0].tool_results.len(), 1);
        assert!(messages[0].tool_results[0].is_error);
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
            disallowed_tools: vec![],
            max_turns: Some(5),
            permission_mode: PermissionMode::Default,
            system_prompt: None,
            max_budget_usd: None,
            working_dir: None,
            mcp_config: None,
            strict_mcp_config: false,
            bare: false,
            resume_session_id: None,
            verbose: false,
            pod_labels: std::collections::BTreeMap::new(),
            inputs: Vec::new(),
            allow_failure: false,
            retry: None,
            _marker: PhantomData,
        };

        let args = build_args(&config).unwrap();

        assert!(args.contains(&"--allowedTools".to_string()));
        assert!(args.contains(&"WebSearch,WebFetch".to_string()));
        assert!(args.contains(&"--json-schema".to_string()));

        let schema_pos = args
            .iter()
            .position(|a| a == "--json-schema")
            .expect("--json-schema missing");
        let schema_value: serde_json::Value =
            serde_json::from_str(&args[schema_pos + 1]).expect("schema is valid JSON");
        assert_eq!(schema_value["type"], "object");
        assert_eq!(schema_value["additionalProperties"], false);

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

    #[test]
    fn parse_response_schema_validation_preserves_usage_data() {
        let stdout = r#"{"session_id":"s1","subtype":"error_max_turns","result":null,"structured_output":null,"usage":{"input_tokens":500,"output_tokens":200,"cache_creation_input_tokens":50,"cache_read_input_tokens":30},"total_cost_usd":0.30,"duration_ms":4500}"#;

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();

        let err = parse_response(stdout, &config, 0).unwrap_err();

        match err {
            AgentError::SchemaValidation { partial_usage, .. } => {
                assert_eq!(partial_usage.cost_usd, Some(0.30));
                assert_eq!(partial_usage.duration_ms, Some(4500));
                assert_eq!(partial_usage.input_tokens, Some(580)); // 500 + 50 + 30
                assert_eq!(partial_usage.output_tokens, Some(200));
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_schema_validation_no_usage_when_parse_fails() {
        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();

        let err = parse_response("not json at all", &config, 0).unwrap_err();

        match err {
            AgentError::SchemaValidation { partial_usage, .. } => {
                assert!(partial_usage.cost_usd.is_none());
                assert!(partial_usage.duration_ms.is_none());
                assert!(partial_usage.input_tokens.is_none());
                assert!(partial_usage.output_tokens.is_none());
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn stream_response_schema_validation_preserves_usage_and_debug() {
        let assistant_line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Working..."}],"stop_reason":"end_turn"}}"#;
        let result_line = r#"{"type":"result","session_id":"s1","subtype":"error_max_turns","result":null,"structured_output":null,"usage":{"input_tokens":300,"output_tokens":100},"total_cost_usd":0.15,"duration_ms":3000}"#;

        let stdout = format!("{assistant_line}\n{result_line}");

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();
        let config = config.verbose(true);

        let err = parse_stream_response(&stdout, &config, 0).unwrap_err();

        match err {
            AgentError::SchemaValidation {
                debug_messages,
                partial_usage,
                ..
            } => {
                assert_eq!(debug_messages.len(), 1);
                assert_eq!(debug_messages[0].text.as_deref(), Some("Working..."));
                assert_eq!(partial_usage.cost_usd, Some(0.15));
                assert_eq!(partial_usage.duration_ms, Some(3000));
                assert_eq!(partial_usage.input_tokens, Some(300));
                assert_eq!(partial_usage.output_tokens, Some(100));
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn handle_nonzero_exit_parses_schema_error_with_usage() {
        let stdout = r#"{"session_id":"s1","subtype":"error_max_turns","result":null,"structured_output":null,"usage":{"input_tokens":100,"output_tokens":50},"total_cost_usd":0.10,"duration_ms":2000}"#;
        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();

        let result = handle_nonzero_exit(1, stdout, "", &config, 2000, "test");

        match result {
            Err(AgentError::SchemaValidation { partial_usage, .. }) => {
                assert_eq!(partial_usage.cost_usd, Some(0.10));
                assert_eq!(partial_usage.duration_ms, Some(2000));
            }
            other => panic!("expected Err(SchemaValidation), got {other:?}"),
        }
    }

    #[test]
    fn parse_response_budget_exceeded_is_its_own_error() {
        let stdout = r#"{"session_id":"s1","subtype":"error_max_budget_usd","result":null,"structured_output":null,"usage":{"input_tokens":500,"output_tokens":200,"cache_creation_input_tokens":50,"cache_read_input_tokens":30},"total_cost_usd":0.30,"duration_ms":4500}"#;

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .max_budget_usd(0.25)
            .into();

        let err = parse_response(stdout, &config, 0).unwrap_err();

        match err {
            AgentError::BudgetExceeded {
                spent_usd,
                limit_usd,
                partial_usage,
                debug_messages,
            } => {
                assert!((spent_usd - 0.30).abs() < f64::EPSILON);
                assert!((limit_usd - 0.25).abs() < f64::EPSILON);
                assert_eq!(partial_usage.cost_usd, Some(0.30));
                assert_eq!(partial_usage.duration_ms, Some(4500));
                assert_eq!(partial_usage.input_tokens, Some(580)); // 500 + 50 + 30
                assert_eq!(partial_usage.output_tokens, Some(200));
                assert!(debug_messages.is_empty());
            }
            other => panic!("expected BudgetExceeded, got {other:?}"),
        }
    }

    #[test]
    fn stream_response_budget_exceeded_keeps_debug_messages() {
        let assistant_line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Working..."}],"stop_reason":"end_turn"}}"#;
        let result_line = r#"{"type":"result","session_id":"s1","subtype":"error_max_budget_usd","result":null,"structured_output":null,"usage":{"input_tokens":300,"output_tokens":100},"total_cost_usd":0.15,"duration_ms":3000}"#;

        let stdout = format!("{assistant_line}\n{result_line}");

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .max_budget_usd(0.10)
            .into();
        let config = config.verbose(true);

        let err = parse_stream_response(&stdout, &config, 0).unwrap_err();

        match err {
            AgentError::BudgetExceeded {
                debug_messages,
                partial_usage,
                ..
            } => {
                assert_eq!(debug_messages.len(), 1);
                assert_eq!(debug_messages[0].text.as_deref(), Some("Working..."));
                assert_eq!(partial_usage.cost_usd, Some(0.15));
            }
            other => panic!("expected BudgetExceeded, got {other:?}"),
        }
    }

    #[test]
    fn handle_nonzero_exit_propagates_budget_exceeded() {
        let stdout = r#"{"session_id":"s1","subtype":"error_max_budget_usd","result":null,"structured_output":null,"usage":{"input_tokens":100,"output_tokens":50},"total_cost_usd":0.10,"duration_ms":2000}"#;
        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .max_budget_usd(0.05)
            .into();

        let result = handle_nonzero_exit(1, stdout, "", &config, 2000, "test");

        match result {
            Err(AgentError::BudgetExceeded { partial_usage, .. }) => {
                assert_eq!(partial_usage.cost_usd, Some(0.10));
                assert_eq!(partial_usage.duration_ms, Some(2000));
            }
            other => panic!("expected Err(BudgetExceeded), got {other:?}"),
        }
    }

    #[test]
    fn handle_nonzero_exit_returns_ok_for_valid_text_response() {
        let stdout = r#"{"result":"Hello!","usage":{"input_tokens":10,"output_tokens":5},"total_cost_usd":0.01,"duration_ms":100}"#;
        let config = AgentConfig::new("test");

        let result = handle_nonzero_exit(1, stdout, "", &config, 100, "test");
        assert!(result.is_ok());
    }

    #[test]
    fn handle_nonzero_exit_falls_back_to_process_failed() {
        let config = AgentConfig::new("test");
        let result = handle_nonzero_exit(1, "", "some random error", &config, 0, "test");

        match result {
            Err(AgentError::ProcessFailed { exit_code, stderr }) => {
                assert_eq!(exit_code, 1);
                assert_eq!(stderr, "some random error");
            }
            other => panic!("expected Err(ProcessFailed), got {other:?}"),
        }
    }

    #[test]
    fn handle_nonzero_exit_prefers_stdout_over_stderr() {
        let stdout =
            r#"{"result":"ok","usage":{"input_tokens":10,"output_tokens":5},"duration_ms":100}"#;
        let stderr = "some error text";
        let config = AgentConfig::new("test");

        let result = handle_nonzero_exit(1, stdout, stderr, &config, 100, "test");
        assert!(result.is_ok());
    }

    #[test]
    fn handle_nonzero_exit_empty_both_returns_no_output() {
        let config = AgentConfig::new("test");
        let result = handle_nonzero_exit(1, "", "", &config, 0, "test");

        match result {
            Err(AgentError::ProcessFailed { stderr, .. }) => {
                assert_eq!(stderr, "(no output captured)");
            }
            other => panic!("expected Err(ProcessFailed), got {other:?}"),
        }
    }

    #[test]
    fn handle_nonzero_exit_truncates_large_stdout_in_error_detail() {
        let large_stdout = "x".repeat(ERROR_DETAIL_MAX_LEN + 10_000);
        let config = AgentConfig::new("test");
        let result = handle_nonzero_exit(1, &large_stdout, "", &config, 0, "test");

        match result {
            Err(AgentError::ProcessFailed { stderr, .. }) => {
                assert!(
                    stderr.len() <= ERROR_DETAIL_MAX_LEN + 20,
                    "error_detail should be truncated, got {} bytes",
                    stderr.len()
                );
                assert!(stderr.ends_with("...(truncated)"));
            }
            other => panic!("expected Err(ProcessFailed), got {other:?}"),
        }
    }

    #[test]
    fn handle_nonzero_exit_truncates_large_stderr_in_error_detail() {
        let large_stderr = "e".repeat(ERROR_DETAIL_MAX_LEN + 5_000);
        let config = AgentConfig::new("test");
        let result = handle_nonzero_exit(1, "", &large_stderr, &config, 0, "test");

        match result {
            Err(AgentError::ProcessFailed { stderr, .. }) => {
                assert!(
                    stderr.len() <= ERROR_DETAIL_MAX_LEN + 20,
                    "error_detail should be truncated, got {} bytes",
                    stderr.len()
                );
                assert!(stderr.ends_with("...(truncated)"));
            }
            other => panic!("expected Err(ProcessFailed), got {other:?}"),
        }
    }

    #[test]
    fn truncate_to_no_truncation_when_short() {
        assert_eq!(truncate_to("hello", 10), "hello");
    }

    #[test]
    fn truncate_to_adds_marker_when_long() {
        let result = truncate_to("abcdefghij", 5);
        assert!(result.starts_with("abcde"));
        assert!(result.ends_with("...(truncated)"));
    }

    #[test]
    fn truncate_to_handles_multibyte_chars() {
        let text = "cafe\u{0301}"; // e + combining accent = 5 bytes
        let result = truncate_to(text, 4);
        assert!(result.ends_with("...(truncated)"));
    }

    #[test]
    fn schema_validation_includes_raw_response_from_result_field() {
        let stdout = r#"{"session_id":"s1","subtype":"success","result":"The model answered with plain text instead of JSON","usage":{"input_tokens":10,"output_tokens":5},"total_cost_usd":0.01,"duration_ms":100}"#;

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();

        let err = parse_response(stdout, &config, 100).unwrap_err();

        match err {
            AgentError::SchemaValidation { raw_response, .. } => {
                let raw = raw_response.expect("raw_response should be Some when result is present");
                assert!(
                    raw.contains("The model answered with plain text instead of JSON"),
                    "raw_response should contain the result text, got: {raw}"
                );
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn schema_validation_raw_response_from_stdout_when_result_null() {
        let stdout = r#"{"session_id":"s1","subtype":"error_max_turns","result":null,"structured_output":null,"usage":{"input_tokens":500,"output_tokens":200},"total_cost_usd":0.30,"duration_ms":4500}"#;

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();

        let err = parse_response(stdout, &config, 0).unwrap_err();

        match err {
            AgentError::SchemaValidation { raw_response, .. } => {
                let raw = raw_response
                    .expect("raw_response should fall back to stdout when result is null");
                assert!(
                    raw.contains("error_max_turns"),
                    "raw_response should contain stdout content, got: {raw}"
                );
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn schema_validation_raw_response_none_when_both_null_and_empty_stdout() {
        let stdout = r#"{"result":null,"structured_output":null}"#;

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();

        let err = parse_response(stdout, &config, 0).unwrap_err();

        match err {
            AgentError::SchemaValidation { raw_response, .. } => {
                // stdout is non-empty (it's the JSON itself), so raw_response falls back to it
                assert!(raw_response.is_some());
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn schema_validation_raw_response_truncated_to_max() {
        let long_text = "x".repeat(5000);
        let stdout = format!(
            r#"{{"result":"{long_text}","structured_output":null,"usage":{{"input_tokens":10,"output_tokens":5}},"total_cost_usd":0.01,"duration_ms":100}}"#,
        );

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{{"type":"object"}}"#)
            .into();

        let err = parse_response(&stdout, &config, 0).unwrap_err();

        match err {
            AgentError::SchemaValidation { raw_response, .. } => {
                let raw = raw_response.expect("raw_response should be present");
                assert!(
                    raw.len() <= RAW_RESPONSE_MAX_LEN + 20,
                    "raw_response should be truncated, got len={}",
                    raw.len()
                );
                assert!(raw.contains("...(truncated)"));
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn stream_schema_validation_preserves_raw_response() {
        let assistant_line = r#"{"type":"assistant","message":{"content":[{"type":"text","text":"Searching..."}],"stop_reason":"end_turn"}}"#;
        let result_line = r#"{"type":"result","session_id":"s1","subtype":"success","result":"text response","usage":{"input_tokens":100,"output_tokens":50},"total_cost_usd":0.01,"duration_ms":500}"#;

        let stdout = format!("{assistant_line}\n{result_line}");

        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();
        let config = config.verbose(true);

        let err = parse_stream_response(&stdout, &config, 500).unwrap_err();

        match err {
            AgentError::SchemaValidation {
                raw_response,
                debug_messages,
                ..
            } => {
                assert!(
                    raw_response.is_some(),
                    "raw_response should be propagated through stream parser"
                );
                assert!(
                    raw_response.unwrap().contains("text response"),
                    "raw_response should contain the result text"
                );
                assert_eq!(debug_messages.len(), 1);
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn parse_response_json_parse_error_includes_raw_stdout() {
        let config: AgentConfig = AgentConfig::new("test")
            .output_schema_raw(r#"{"type":"object"}"#)
            .into();

        let err = parse_response("this is not JSON at all", &config, 0).unwrap_err();

        match err {
            AgentError::SchemaValidation { raw_response, .. } => {
                let raw = raw_response
                    .expect("raw_response should contain the raw stdout on parse failure");
                assert!(
                    raw.contains("this is not JSON at all"),
                    "raw_response should contain the unparseable input, got: {raw}"
                );
            }
            other => panic!("expected SchemaValidation, got {other:?}"),
        }
    }

    #[test]
    fn extract_raw_response_text_prefers_result_string() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": "The agent said hello",
            "structured_output": null,
        }))
        .unwrap();
        let raw = extract_raw_response_text(&parsed, "full stdout...");
        assert_eq!(raw, Some("The agent said hello".to_string()));
    }

    #[test]
    fn extract_raw_response_text_falls_back_to_stdout() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": null,
            "structured_output": null,
        }))
        .unwrap();
        let raw = extract_raw_response_text(&parsed, "raw stdout content");
        assert_eq!(raw, Some("raw stdout content".to_string()));
    }

    #[test]
    fn extract_raw_response_text_none_when_all_empty() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": null,
            "structured_output": null,
        }))
        .unwrap();
        let raw = extract_raw_response_text(&parsed, "");
        assert!(raw.is_none());
    }

    #[test]
    fn extract_raw_response_text_stringifies_non_string_result() {
        let parsed: ClaudeJsonOutput = serde_json::from_value(json!({
            "result": {"partial": "data", "count": 42},
            "structured_output": null,
        }))
        .unwrap();
        let raw = extract_raw_response_text(&parsed, "");
        let raw = raw.expect("should extract stringified JSON object");
        assert!(raw.contains("partial"));
        assert!(raw.contains("42"));
    }

    #[test]
    fn build_command_small_prompt_uses_arg() {
        let config = AgentConfig::new("hello world");
        let built = build_command(&config).unwrap();
        assert!(built.stdin_prompt.is_none());
        assert_eq!(built.args[0], "-p");
        assert_eq!(built.args[1], "hello world");
    }

    #[test]
    fn build_command_large_prompt_uses_stdin() {
        let big = "x".repeat(PROMPT_STDIN_THRESHOLD + 1);
        let config = AgentConfig::new(&big);
        let built = build_command(&config).unwrap();
        assert!(built.stdin_prompt.is_some());
        assert_eq!(built.stdin_prompt.as_ref().unwrap().len(), big.len());
        assert!(!built.args.contains(&"-p".to_string()));
    }

    #[test]
    fn build_command_exact_threshold_uses_arg() {
        let exact = "y".repeat(PROMPT_STDIN_THRESHOLD);
        let config = AgentConfig::new(&exact);
        let built = build_command(&config).unwrap();
        assert!(built.stdin_prompt.is_none());
        assert_eq!(built.args[0], "-p");
    }

    #[test]
    fn build_command_preserves_other_flags() {
        let big = "z".repeat(PROMPT_STDIN_THRESHOLD + 1);
        let config = AgentConfig::new(&big)
            .model("claude-sonnet-4-20250514")
            .max_turns(5);
        let built = build_command(&config).unwrap();
        assert!(built.stdin_prompt.is_some());
        assert!(built.args.contains(&"--model".to_string()));
        assert!(built.args.contains(&"claude-sonnet-4-20250514".to_string()));
        assert!(built.args.contains(&"--max-turns".to_string()));
        assert!(built.args.contains(&"5".to_string()));
    }
}
