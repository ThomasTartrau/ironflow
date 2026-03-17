//! Claude Code CLI provider.
//!
//! [`ClaudeCodeProvider`] is the default [`AgentProvider`]
//! implementation. It invokes the `claude` CLI in headless mode with
//! `claude -p <prompt> --output-format json`, parses the JSON response, and
//! returns an [`AgentOutput`] with the result,
//! session metadata, and token/cost statistics.
//!
//! # Requirements
//!
//! The `claude` binary must be available on `$PATH`. Install it via
//! `npm install -g @anthropic-ai/claude-code`.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_core::prelude::*;
//!
//! # async fn example() -> Result<(), OperationError> {
//! let provider = ClaudeCodeProvider::new();
//!
//! let result = Agent::new()
//!     .prompt("What is 2 + 2?")
//!     .run(&provider)
//!     .await?;
//!
//! println!("{}", result.text());
//! # Ok(())
//! # }
//! ```

use serde::Deserialize;
use serde_json::{Map, Value, from_str};
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tracing::{debug, error, warn};

use crate::error::AgentError;
use crate::operations::agent::PermissionMode;
use crate::provider::{AgentConfig, AgentOutput, AgentProvider, InvokeFuture};
use crate::utils::truncate_output;

/// Default timeout for a single Claude CLI invocation (5 minutes).
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(300);

/// [`AgentProvider`] that shells out to the
/// `claude` CLI.
///
/// The provider spawns a `claude` child process for each invocation, passing
/// the prompt and configuration as command-line arguments. The `CLAUDECODE`
/// environment variable is removed to avoid recursive invocation when running
/// inside Claude Code itself.
#[derive(Clone)]
pub struct ClaudeCodeProvider {
    /// Maximum wall-clock time to wait for the `claude` process.
    pub(crate) timeout: Duration,
}

impl ClaudeCodeProvider {
    /// Create a new provider with the default timeout of 5 minutes.
    pub fn new() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Override the default timeout.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use ironflow_core::providers::claude::ClaudeCodeProvider;
    ///
    /// let provider = ClaudeCodeProvider::new()
    ///     .timeout(Duration::from_secs(600));
    /// ```
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

impl Default for ClaudeCodeProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Deserialize)]
struct ClaudeJsonOutput {
    session_id: Option<String>,
    subtype: Option<String>,
    result: Option<Value>,
    structured_output: Option<Value>,
    usage: Option<ClaudeUsage>,
    total_cost_usd: Option<f64>,
    duration_ms: Option<u64>,
    #[serde(rename = "modelUsage")]
    model_usage: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
struct ClaudeUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

impl ClaudeUsage {
    fn total_input_tokens(&self) -> u64 {
        self.input_tokens.unwrap_or(0)
            + self.cache_creation_input_tokens.unwrap_or(0)
            + self.cache_read_input_tokens.unwrap_or(0)
    }

    fn total_output_tokens(&self) -> u64 {
        self.output_tokens.unwrap_or(0)
    }
}

fn push_flag(args: &mut Vec<String>, flag: &str, value: &str) {
    args.push(flag.to_string());
    args.push(value.to_string());
}

fn push_opt(args: &mut Vec<String>, flag: &str, value: &Option<impl ToString>) {
    if let Some(v) = value {
        push_flag(args, flag, &v.to_string());
    }
}

/// Extract a structured JSON value from a parsed Claude CLI response.
///
/// Prefers `structured_output`; falls back to parsing `result` as JSON
/// (direct parse, code-fence extraction, or brace extraction).
fn extract_structured_value(parsed: &ClaudeJsonOutput) -> Option<Value> {
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

impl AgentProvider for ClaudeCodeProvider {
    fn invoke<'a>(&'a self, config: &'a AgentConfig) -> InvokeFuture<'a> {
        Box::pin(async move {
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
                        stderr: "BypassPermissions requires IRONFLOW_ALLOW_BYPASS=1 environment variable".to_string(),
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

            debug!(
                model = %config.model,
                has_system_prompt = config.system_prompt.is_some(),
                has_json_schema = config.json_schema.is_some(),
                permission_mode = ?config.permission_mode,
                "spawning claude process"
            );

            let start = Instant::now();

            let mut cmd = Command::new("claude");
            cmd.args(&args)
                .env_remove("CLAUDECODE")
                .env_remove("IRONFLOW_ALLOW_BYPASS")
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true);

            if let Some(ref dir) = config.working_dir {
                cmd.current_dir(dir);
            }

            let child = cmd.spawn().map_err(|e| AgentError::ProcessFailed {
                exit_code: -1,
                stderr: format!("failed to spawn claude: {e}"),
            })?;

            let output = match tokio::time::timeout(self.timeout, child.wait_with_output()).await {
                Ok(result) => result.map_err(|e| AgentError::ProcessFailed {
                    exit_code: -1,
                    stderr: format!("failed to wait for claude: {e}"),
                })?,
                Err(_) => {
                    warn!(timeout = ?self.timeout, "claude process timed out");
                    return Err(AgentError::Timeout {
                        limit: self.timeout,
                    });
                }
            };

            let duration_ms = start.elapsed().as_millis() as u64;

            if !output.status.success() {
                let stderr = truncate_output(&output.stderr, "claude stderr");
                let exit_code = output.status.code().unwrap_or(-1);
                error!(
                    exit_code,
                    stderr_len = stderr.len(),
                    "claude process failed"
                );
                return Err(AgentError::ProcessFailed { exit_code, stderr });
            }

            let stdout = truncate_output(&output.stdout, "claude stdout");
            debug!(stdout_len = stdout.len(), "claude process completed");

            let parsed: ClaudeJsonOutput =
                from_str(&stdout).map_err(|e| AgentError::SchemaValidation {
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
                // Filter out null: serde deserializes JSON null as Some(Value::Null),
                // not None, so unwrap_or_else alone doesn't catch it
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
                duration_ms: parsed.duration_ms.unwrap_or(duration_ms),
            })
        })
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
        // serde deserializes `"result": null` as None for Option<Value>
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
    fn provider_default_timeout() {
        let provider = ClaudeCodeProvider::new();
        assert_eq!(provider.timeout, DEFAULT_TIMEOUT);
    }

    #[test]
    fn provider_custom_timeout() {
        let provider = ClaudeCodeProvider::new().timeout(Duration::from_secs(600));
        assert_eq!(provider.timeout, Duration::from_secs(600));
    }

    #[test]
    fn provider_default_matches_new() {
        let from_new = ClaudeCodeProvider::new();
        let from_default = ClaudeCodeProvider::default();
        assert_eq!(from_new.timeout, from_default.timeout);
    }

    #[test]
    fn provider_clone() {
        let provider = ClaudeCodeProvider::new().timeout(Duration::from_secs(42));
        let cloned = provider.clone();
        assert_eq!(cloned.timeout, Duration::from_secs(42));
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
}
