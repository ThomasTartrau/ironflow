//! OpenAI-compatible adapter implementation shared by OpenAI and Mistral.

use serde_json::{Map, Value, json};

use crate::error::AgentError;
use crate::operations::agent::Model;
use crate::pricing::{CostBreakdown, StaticPricing};
use crate::provider::AgentConfig;
use crate::providers::http::adapter::{HttpAgentAdapter, HttpToolCall, HttpUsage, TurnResult};
use crate::providers::http::sse::SseDelta;
use crate::schema_transform::transform_schema;

use super::config::OpenAiCompatConfig;

/// Adapter for OpenAI-compatible chat completion APIs.
///
/// Parameterized by a [`OpenAiCompatConfig`] to support both OpenAI and Mistral
/// with shared request/response logic.
pub struct OpenAiCompatAdapter<C: OpenAiCompatConfig> {
    config: C,
}

impl<C: OpenAiCompatConfig> OpenAiCompatAdapter<C> {
    /// Create a new adapter with the given configuration.
    pub fn new(config: C) -> Self {
        Self { config }
    }
}

impl<C: OpenAiCompatConfig> HttpAgentAdapter for OpenAiCompatAdapter<C> {
    fn provider_name(&self) -> &'static str {
        self.config.provider_name()
    }

    fn endpoint_url(&self, _model: &str) -> String {
        format!("{}/chat/completions", self.config.base_url())
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        vec![(
            "Authorization".to_string(),
            format!("Bearer {}", self.config.api_key()),
        )]
    }

    fn build_request(&self, config: &AgentConfig) -> Result<Value, AgentError> {
        let model = self.resolve_model(&config.model);

        let mut messages: Vec<Value> = Vec::new();

        if let Some(ref system) = config.system_prompt {
            messages.push(json!({
                "role": "system",
                "content": system
            }));
        }

        messages.push(json!({
            "role": "user",
            "content": config.prompt
        }));

        let mut body = json!({
            "model": model,
            "messages": messages
        });

        if config.verbose {
            body["stream"] = json!(true);
        }

        if let Some(ref schema_str) = config.json_schema {
            let transformed = transform_schema(schema_str);
            let schema_value: Value = serde_json::from_str(&transformed).unwrap_or(json!({}));

            if self.config.supports_json_schema() {
                body["response_format"] = json!({
                    "type": "json_schema",
                    "json_schema": {
                        "name": "output",
                        "schema": schema_value,
                        "strict": true
                    }
                });
            } else {
                body["response_format"] = json!({ "type": "json_object" });
            }
        }

        Ok(body)
    }

    fn parse_response(&self, body: &Value, config: &AgentConfig) -> Result<TurnResult, AgentError> {
        let choice = body
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"));

        let message = choice.ok_or_else(|| AgentError::HttpProvider {
            provider: self.config.provider_name().to_string(),
            status_code: 0,
            message: "no choices in response".to_string(),
        })?;

        let text = message
            .get("content")
            .and_then(|c| c.as_str())
            .map(String::from);

        let tool_calls = parse_tool_calls(message);

        let finish_reason = body
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("finish_reason"))
            .and_then(|f| f.as_str())
            .unwrap_or("stop");

        let is_final = finish_reason == "stop" || finish_reason == "length";

        let structured_value = if config.json_schema.is_some() {
            text.as_deref().and_then(parse_json_content)
        } else {
            None
        };

        let usage = parse_usage(body);
        let model = body.get("model").and_then(|m| m.as_str()).map(String::from);

        Ok(TurnResult {
            text: if structured_value.is_some() {
                None
            } else {
                text
            },
            tool_calls,
            is_final,
            structured_value,
            usage,
            model,
        })
    }

    fn parse_sse_line(&self, line: &str) -> Option<SseDelta> {
        if line == "[DONE]" {
            return Some(SseDelta::Done);
        }

        let data: Value = serde_json::from_str(line).ok()?;

        if let Some(usage) = data.get("usage") {
            let input = usage
                .get("prompt_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output = usage
                .get("completion_tokens")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if input > 0 || output > 0 {
                return Some(SseDelta::Usage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }
        }

        let delta = data.get("choices")?.get(0)?.get("delta")?;

        if let Some(content) = delta.get("content").and_then(|c| c.as_str())
            && !content.is_empty()
        {
            return Some(SseDelta::Text(content.to_string()));
        }

        if let Some(tc) = delta
            .get("tool_calls")
            .and_then(|t| t.as_array())
            .and_then(|arr| arr.first())
        {
            let index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
            let id = tc.get("id").and_then(|i| i.as_str()).map(String::from);
            let name = tc
                .get("function")
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .map(String::from);
            let args = tc
                .get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .unwrap_or("");
            return Some(SseDelta::ToolCallDelta {
                index,
                id,
                name,
                args_fragment: args.to_string(),
            });
        }

        None
    }

    fn fold_sse_deltas(
        &self,
        deltas: Vec<SseDelta>,
        config: &AgentConfig,
    ) -> Result<TurnResult, AgentError> {
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_builders: Vec<ToolCallBuilder> = Vec::new();
        let mut usage = HttpUsage::default();

        for delta in deltas {
            match delta {
                SseDelta::Text(t) => text_parts.push(t),
                SseDelta::ToolCallDelta {
                    index,
                    id,
                    name,
                    args_fragment,
                } => {
                    while tool_builders.len() <= index {
                        tool_builders.push(ToolCallBuilder::default());
                    }
                    let builder = &mut tool_builders[index];
                    if let Some(i) = id {
                        builder.id = Some(i);
                    }
                    if let Some(n) = name {
                        builder.name = Some(n);
                    }
                    builder.args.push_str(&args_fragment);
                }
                SseDelta::Usage {
                    input_tokens,
                    output_tokens,
                } => {
                    usage.input_tokens = Some(input_tokens);
                    usage.output_tokens = Some(output_tokens);
                }
                SseDelta::Done | SseDelta::StructuredValue(_) => {}
            }
        }

        let full_text = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let tool_calls: Vec<HttpToolCall> = tool_builders
            .into_iter()
            .filter_map(|b| {
                Some(HttpToolCall {
                    id: b.id?,
                    name: b.name?,
                    input: serde_json::from_str(&b.args).unwrap_or(Value::Object(Map::new())),
                })
            })
            .collect();

        let is_final = tool_calls.is_empty();

        let structured_value = if config.json_schema.is_some() {
            full_text.as_deref().and_then(parse_json_content)
        } else {
            None
        };

        Ok(TurnResult {
            text: if structured_value.is_some() {
                None
            } else {
                full_text
            },
            tool_calls,
            is_final,
            structured_value,
            usage,
            model: None,
        })
    }

    fn compute_cost(&self, model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
        let pricing = StaticPricing::new();
        let bd = CostBreakdown::compute(&pricing, model, input_tokens, output_tokens);
        Some(bd.total_usd)
    }

    fn resolve_model(&self, model: &str) -> String {
        match model {
            m if m == Model::SONNET || m == Model::OPUS => self.config.default_model().to_string(),
            m if m == Model::HAIKU => self.config.small_model().to_string(),
            other => other.to_string(),
        }
    }
}

/// Strip markdown code fences from a string if present.
///
/// Handles both `` ```json ... ``` `` and `` ``` ... ``` `` patterns
/// that some providers (NVIDIA, Mistral) wrap around JSON output.
fn strip_code_fences(s: &str) -> &str {
    let trimmed = s.trim();
    let after_open = if trimmed.starts_with("```json\n") || trimmed.starts_with("```json\r") {
        trimmed["```json".len()..].trim_start()
    } else if trimmed.starts_with("```\n") || trimmed.starts_with("```\r") {
        trimmed["```".len()..].trim_start()
    } else {
        return trimmed;
    };
    match after_open.strip_suffix("```") {
        Some(inner) => inner.trim_end(),
        None => trimmed,
    }
}

/// Try to parse JSON content, stripping code fences if necessary.
///
/// First attempts direct parsing, then retries after stripping markdown
/// code fences that some providers wrap around structured output.
fn parse_json_content(s: &str) -> Option<Value> {
    serde_json::from_str::<Value>(s)
        .ok()
        .or_else(|| serde_json::from_str::<Value>(strip_code_fences(s)).ok())
}

fn parse_tool_calls(message: &Value) -> Vec<HttpToolCall> {
    message
        .get("tool_calls")
        .and_then(|tc| tc.as_array())
        .map(|calls| {
            calls
                .iter()
                .filter_map(|call| {
                    let id = call.get("id")?.as_str()?.to_string();
                    let function = call.get("function")?;
                    let name = function.get("name")?.as_str()?.to_string();
                    let args_str = function.get("arguments")?.as_str()?;
                    let input = serde_json::from_str(args_str).unwrap_or(Value::Object(Map::new()));
                    Some(HttpToolCall { id, name, input })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_usage(body: &Value) -> HttpUsage {
    let usage = body.get("usage");
    HttpUsage {
        input_tokens: usage
            .and_then(|u| u.get("prompt_tokens"))
            .and_then(|v| v.as_u64()),
        output_tokens: usage
            .and_then(|u| u.get("completion_tokens"))
            .and_then(|v| v.as_u64()),
    }
}

#[derive(Default)]
struct ToolCallBuilder {
    id: Option<String>,
    name: Option<String>,
    args: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::providers::http::openai_compat::config::{
        MistralConfig, NvidiaConfig, OpenAiConfig,
    };

    fn openai_adapter() -> OpenAiCompatAdapter<OpenAiConfig> {
        OpenAiCompatAdapter::new(OpenAiConfig::new(
            "test-key".to_string(),
            "https://api.openai.com/v1".to_string(),
        ))
    }

    fn mistral_adapter() -> OpenAiCompatAdapter<MistralConfig> {
        OpenAiCompatAdapter::new(MistralConfig::new("test-key".to_string()))
    }

    #[test]
    fn build_request_basic() {
        let adapter = openai_adapter();
        let config = AgentConfig::new("Hello world");
        let body = adapter.build_request(&config).expect("test");

        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello world");
        assert!(body.get("stream").is_none());
        assert!(body.get("response_format").is_none());
    }

    #[test]
    fn build_request_with_system_prompt() {
        let adapter = openai_adapter();
        let config = AgentConfig::new("Hello").system_prompt("You are helpful");
        let body = adapter.build_request(&config).expect("test");

        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][0]["content"], "You are helpful");
        assert_eq!(body["messages"][1]["role"], "user");
        assert_eq!(body["messages"][1]["content"], "Hello");
    }

    #[test]
    fn build_request_with_json_schema_openai() {
        let adapter = openai_adapter();
        let schema = r#"{"type":"object","properties":{"name":{"type":"string"}}}"#;
        let config = AgentConfig::new("Give name")
            .output_schema_raw(schema)
            .into();
        let body = adapter.build_request(&config).expect("test");

        let rf = &body["response_format"];
        assert_eq!(rf["type"], "json_schema");
        assert_eq!(rf["json_schema"]["name"], "output");
        assert_eq!(rf["json_schema"]["strict"], true);
    }

    #[test]
    fn build_request_with_json_schema_mistral_uses_json_object() {
        let adapter = mistral_adapter();
        let schema = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;
        let config = AgentConfig::new("Give x").output_schema_raw(schema).into();
        let body = adapter.build_request(&config).expect("test");

        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn parse_response_text() {
        let adapter = openai_adapter();
        let body = json!({
            "choices": [{"message": {"content": "Hello!"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5},
            "model": "gpt-5.5"
        });
        let config = AgentConfig::new("Hi");
        let result = adapter.parse_response(&body, &config).expect("test");

        assert_eq!(result.text.as_deref(), Some("Hello!"));
        assert!(result.is_final);
        assert!(result.structured_value.is_none());
        assert_eq!(result.usage.input_tokens, Some(10));
        assert_eq!(result.usage.output_tokens, Some(5));
        assert_eq!(result.model.as_deref(), Some("gpt-5.5"));
    }

    #[test]
    fn parse_response_structured() {
        let adapter = openai_adapter();
        let body = json!({
            "choices": [{"message": {"content": "{\"name\":\"Alice\"}"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 8},
            "model": "gpt-5.5"
        });
        let schema = r#"{"type":"object"}"#;
        let config = AgentConfig::new("Name?").output_schema_raw(schema).into();
        let result = adapter.parse_response(&body, &config).expect("test");

        assert!(result.text.is_none());
        assert_eq!(result.structured_value, Some(json!({"name": "Alice"})));
    }

    #[test]
    fn parse_response_tool_calls() {
        let adapter = openai_adapter();
        let body = json!({
            "choices": [{"message": {
                "content": null,
                "tool_calls": [{"id": "call_1", "function": {"name": "get_weather", "arguments": "{\"city\":\"Paris\"}"}}]
            }, "finish_reason": "tool_calls"}],
            "usage": {"prompt_tokens": 20, "completion_tokens": 15},
            "model": "gpt-5.5"
        });
        let config = AgentConfig::new("Weather");
        let result = adapter.parse_response(&body, &config).expect("test");

        assert!(!result.is_final);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "get_weather");
        assert_eq!(result.tool_calls[0].input, json!({"city": "Paris"}));
    }

    #[test]
    fn parse_sse_line_text_delta() {
        let adapter = openai_adapter();
        let line = r#"{"choices":[{"delta":{"content":"Hello"}}]}"#;
        let delta = adapter.parse_sse_line(line).expect("test");
        assert!(matches!(delta, SseDelta::Text(t) if t == "Hello"));
    }

    #[test]
    fn parse_sse_line_done() {
        let adapter = openai_adapter();
        let delta = adapter.parse_sse_line("[DONE]").expect("test");
        assert!(matches!(delta, SseDelta::Done));
    }

    #[test]
    fn parse_sse_line_usage() {
        let adapter = openai_adapter();
        let line = r#"{"usage":{"prompt_tokens":100,"completion_tokens":50}}"#;
        let delta = adapter.parse_sse_line(line).expect("test");
        assert!(matches!(
            delta,
            SseDelta::Usage {
                input_tokens: 100,
                output_tokens: 50
            }
        ));
    }

    #[test]
    fn resolve_model_aliases() {
        let adapter = openai_adapter();
        assert_eq!(adapter.resolve_model("sonnet"), "gpt-5.5");
        assert_eq!(adapter.resolve_model("opus"), "gpt-5.5");
        assert_eq!(adapter.resolve_model("haiku"), "gpt-5.4-mini");
        assert_eq!(adapter.resolve_model("gpt-4.1"), "gpt-4.1");
    }

    #[test]
    fn resolve_model_mistral_aliases() {
        let adapter = mistral_adapter();
        assert_eq!(adapter.resolve_model("sonnet"), "mistral-medium-3.5");
        assert_eq!(adapter.resolve_model("haiku"), "mistral-small-latest");
    }

    #[test]
    fn endpoint_url_format() {
        let adapter = openai_adapter();
        assert_eq!(
            adapter.endpoint_url("gpt-5.5"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn auth_headers_bearer() {
        let adapter = openai_adapter();
        let headers = adapter.auth_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "Authorization");
        assert_eq!(headers[0].1, "Bearer test-key");
    }

    #[test]
    fn fold_sse_deltas_text() {
        let adapter = openai_adapter();
        let deltas = vec![
            SseDelta::Text("Hel".to_string()),
            SseDelta::Text("lo".to_string()),
            SseDelta::Usage {
                input_tokens: 10,
                output_tokens: 5,
            },
            SseDelta::Done,
        ];
        let config = AgentConfig::new("Hi");
        let result = adapter.fold_sse_deltas(deltas, &config).expect("test");

        assert_eq!(result.text.as_deref(), Some("Hello"));
        assert_eq!(result.usage.input_tokens, Some(10));
        assert_eq!(result.usage.output_tokens, Some(5));
    }

    fn nvidia_adapter() -> OpenAiCompatAdapter<NvidiaConfig> {
        OpenAiCompatAdapter::new(NvidiaConfig::new("nvapi-test-key".to_string()))
    }

    #[test]
    fn nvidia_endpoint_url() {
        let adapter = nvidia_adapter();
        assert_eq!(
            adapter.endpoint_url("z-ai/glm-5.1"),
            "https://integrate.api.nvidia.com/v1/chat/completions"
        );
    }

    #[test]
    fn nvidia_auth_headers() {
        let adapter = nvidia_adapter();
        let headers = adapter.auth_headers();
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].0, "Authorization");
        assert_eq!(headers[0].1, "Bearer nvapi-test-key");
    }

    #[test]
    fn nvidia_resolve_model_aliases() {
        let adapter = nvidia_adapter();
        assert_eq!(
            adapter.resolve_model("sonnet"),
            "deepseek-ai/deepseek-v4-flash"
        );
        assert_eq!(
            adapter.resolve_model("haiku"),
            "nvidia/nvidia-nemotron-nano-9b-v2"
        );
        assert_eq!(adapter.resolve_model("z-ai/glm-5.1"), "z-ai/glm-5.1");
    }

    #[test]
    fn nvidia_build_request_uses_json_object() {
        let adapter = nvidia_adapter();
        let schema = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;
        let config = AgentConfig::new("Give x").output_schema_raw(schema).into();
        let body = adapter.build_request(&config).expect("test");

        assert_eq!(body["response_format"]["type"], "json_object");
    }

    #[test]
    fn nvidia_build_request_default_model() {
        let adapter = nvidia_adapter();
        let config = AgentConfig::new("Hello");
        let body = adapter.build_request(&config).expect("test");

        assert_eq!(body["model"], "deepseek-ai/deepseek-v4-flash");
    }

    #[test]
    fn nvidia_build_request_explicit_model() {
        let adapter = nvidia_adapter();
        let config = AgentConfig::new("Hello").model("z-ai/glm-5.1");
        let body = adapter.build_request(&config).expect("test");

        assert_eq!(body["model"], "z-ai/glm-5.1");
    }

    #[test]
    fn strip_code_fences_json_tag() {
        let input = "```json\n{\"action\": \"query\"}\n```";
        assert_eq!(strip_code_fences(input), "{\"action\": \"query\"}");
    }

    #[test]
    fn strip_code_fences_no_tag() {
        let input = "```\n{\"action\": \"query\"}\n```";
        assert_eq!(strip_code_fences(input), "{\"action\": \"query\"}");
    }

    #[test]
    fn strip_code_fences_passthrough_plain_json() {
        let input = "{\"action\": \"query\"}";
        assert_eq!(strip_code_fences(input), input);
    }

    #[test]
    fn strip_code_fences_with_surrounding_whitespace() {
        let input = "  \n```json\n{\"action\": \"query\"}\n```\n  ";
        assert_eq!(strip_code_fences(input), "{\"action\": \"query\"}");
    }

    #[test]
    fn strip_code_fences_no_closing_fence() {
        let input = "```json\n{\"action\": \"query\"}";
        assert_eq!(strip_code_fences(input), input.trim());
    }

    #[test]
    fn parse_json_content_plain() {
        let result = parse_json_content("{\"name\": \"Alice\"}");
        assert_eq!(result, Some(json!({"name": "Alice"})));
    }

    #[test]
    fn parse_json_content_with_code_fences() {
        let input = "```json\n{\"name\": \"Alice\"}\n```";
        let result = parse_json_content(input);
        assert_eq!(result, Some(json!({"name": "Alice"})));
    }

    #[test]
    fn parse_json_content_with_bare_fences() {
        let input = "```\n{\"x\": 42}\n```";
        let result = parse_json_content(input);
        assert_eq!(result, Some(json!({"x": 42})));
    }

    #[test]
    fn parse_json_content_invalid() {
        assert_eq!(parse_json_content("not json at all"), None);
    }

    #[test]
    fn parse_response_structured_with_code_fences() {
        let adapter = nvidia_adapter();
        let body = json!({
            "choices": [{"message": {"content": "```json\n{\"action\":\"query\",\"params\":{\"domain\":\"grafana\"}}\n```"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": 10, "completion_tokens": 8},
            "model": "deepseek-ai/deepseek-v4-flash"
        });
        let schema = r#"{"type":"object"}"#;
        let config = AgentConfig::new("Action?").output_schema_raw(schema).into();
        let result = adapter.parse_response(&body, &config).expect("test");

        assert!(result.text.is_none());
        assert_eq!(
            result.structured_value,
            Some(json!({"action": "query", "params": {"domain": "grafana"}}))
        );
    }

    #[test]
    fn fold_sse_deltas_structured_with_code_fences() {
        let adapter = nvidia_adapter();
        let deltas = vec![
            SseDelta::Text("```json\n".to_string()),
            SseDelta::Text("{\"action\":\"query\"}".to_string()),
            SseDelta::Text("\n```".to_string()),
            SseDelta::Done,
        ];
        let schema = r#"{"type":"object"}"#;
        let config = AgentConfig::new("Action?").output_schema_raw(schema).into();
        let result = adapter.fold_sse_deltas(deltas, &config).expect("test");

        assert!(result.text.is_none());
        assert_eq!(result.structured_value, Some(json!({"action": "query"})));
    }
}
