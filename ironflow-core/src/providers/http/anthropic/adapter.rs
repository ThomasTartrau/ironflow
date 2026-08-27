//! Anthropic Messages API adapter implementation.

use serde_json::{Value, json};

use crate::error::AgentError;
use crate::operations::agent::Model;
use crate::pricing::{CostBreakdown, StaticPricing};
use crate::provider::AgentConfig;
use crate::providers::http::adapter::{HttpAgentAdapter, HttpToolCall, HttpUsage, TurnResult};
use crate::providers::http::sse::SseDelta;
use crate::schema_transform::transform_schema;

/// Known Anthropic model identifiers for the Messages API.
pub struct AnthropicModel;

impl AnthropicModel {
    /// Claude Fable 5 - most capable widely released model (1M context).
    pub const FABLE_5: &str = "claude-fable-5";
    /// Claude Mythos 5 - Fable 5 capabilities, limited availability (Project Glasswing, 1M context).
    pub const MYTHOS_5: &str = "claude-mythos-5";
    /// Claude Opus 5 - flagship for complex agentic coding and enterprise work (1M context).
    pub const OPUS_5: &str = "claude-opus-5";
    /// Claude Sonnet 5 - best combination of speed and intelligence (1M context).
    pub const SONNET_5: &str = "claude-sonnet-5";
    /// Claude Opus 4.8 - previous Opus flagship (1M context).
    pub const OPUS_4_8: &str = "claude-opus-4-8";
    /// Claude Opus 4.7 - previous generation flagship (1M context).
    pub const OPUS_4_7: &str = "claude-opus-4-7";
    /// Claude Sonnet 4.6 - best combination of speed and intelligence (1M context).
    pub const SONNET_4_6: &str = "claude-sonnet-4-6";
    /// Claude Haiku 4.5 - fastest model with near-frontier intelligence (200k context).
    pub const HAIKU_4_5: &str = "claude-haiku-4-5-20251001";
    /// Claude Opus 4.6 - previous generation most capable (1M context).
    pub const OPUS_4_6: &str = "claude-opus-4-6";
    /// Claude Sonnet 4.5 - previous generation balanced (200k context).
    pub const SONNET_4_5: &str = "claude-sonnet-4-5-20250929";
}

/// Adapter for the Anthropic Messages API (`/v1/messages`).
///
/// Uses tool forcing for structured output: defines a synthetic `output` tool
/// with the requested JSON schema and forces the model to call it.
pub struct AnthropicApiAdapter {
    api_key: String,
    anthropic_version: String,
    default_model: String,
}

impl AnthropicApiAdapter {
    /// Create a new adapter with explicit credentials.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            anthropic_version: "2023-06-01".to_string(),
            default_model: AnthropicModel::SONNET_5.to_string(),
        }
    }

    /// Override the default model.
    pub fn with_default_model(mut self, model: String) -> Self {
        self.default_model = model;
        self
    }
}

impl HttpAgentAdapter for AnthropicApiAdapter {
    fn provider_name(&self) -> &'static str {
        "anthropic"
    }

    fn endpoint_url(&self, _model: &str) -> String {
        "https://api.anthropic.com/v1/messages".to_string()
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        vec![
            ("x-api-key".to_string(), self.api_key.clone()),
            (
                "anthropic-version".to_string(),
                self.anthropic_version.clone(),
            ),
            ("content-type".to_string(), "application/json".to_string()),
        ]
    }

    fn build_request(&self, config: &AgentConfig) -> Result<Value, AgentError> {
        let model = self.resolve_model(&config.model);

        let messages = vec![json!({
            "role": "user",
            "content": config.prompt
        })];

        let mut body = json!({
            "model": model,
            "max_tokens": 8192,
            "messages": messages
        });

        if let Some(ref system) = config.system_prompt {
            body["system"] = json!(system);
        }

        if config.verbose {
            body["stream"] = json!(true);
        }

        if let Some(ref schema_str) = config.json_schema {
            let transformed = transform_schema(schema_str);
            let schema_value: Value = serde_json::from_str(&transformed).unwrap_or(json!({}));

            body["tools"] = json!([{
                "name": "structured_output",
                "description": "Output the result in the requested JSON schema.",
                "input_schema": schema_value
            }]);
            body["tool_choice"] = json!({
                "type": "tool",
                "name": "structured_output"
            });
        }

        Ok(body)
    }

    fn parse_response(&self, body: &Value, config: &AgentConfig) -> Result<TurnResult, AgentError> {
        let content = body.get("content").and_then(|c| c.as_array());

        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<HttpToolCall> = Vec::new();
        let mut structured_value: Option<Value> = None;

        if let Some(blocks) = content {
            for block in blocks {
                let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match block_type {
                    "text" => {
                        if let Some(t) = block.get("text").and_then(|t| t.as_str()) {
                            text_parts.push(t.to_string());
                        }
                    }
                    "tool_use" => {
                        let name = block
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string();
                        let id = block
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or("")
                            .to_string();
                        let input = block.get("input").cloned().unwrap_or(json!({}));

                        if config.json_schema.is_some() && name == "structured_output" {
                            structured_value = Some(input);
                        } else {
                            tool_calls.push(HttpToolCall { id, name, input });
                        }
                    }
                    _ => {}
                }
            }
        }

        let stop_reason = body
            .get("stop_reason")
            .and_then(|s| s.as_str())
            .unwrap_or("end_turn");

        let is_final = stop_reason == "end_turn" || stop_reason == "max_tokens";

        let usage = parse_anthropic_usage(body);
        let model = body.get("model").and_then(|m| m.as_str()).map(String::from);

        let text = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        Ok(TurnResult {
            text,
            tool_calls,
            is_final,
            structured_value,
            usage,
            model,
        })
    }

    fn parse_sse_line(&self, line: &str) -> Option<SseDelta> {
        let data: Value = serde_json::from_str(line).ok()?;
        let event_type = data.get("type").and_then(|t| t.as_str())?;

        match event_type {
            "content_block_delta" => {
                let delta = data.get("delta")?;
                let delta_type = delta.get("type").and_then(|t| t.as_str())?;
                match delta_type {
                    "text_delta" => {
                        let text = delta.get("text").and_then(|t| t.as_str())?;
                        Some(SseDelta::Text(text.to_string()))
                    }
                    "input_json_delta" => {
                        let partial = delta
                            .get("partial_json")
                            .and_then(|p| p.as_str())
                            .unwrap_or("");
                        let index =
                            data.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                        Some(SseDelta::ToolCallDelta {
                            index,
                            id: None,
                            name: None,
                            args_fragment: partial.to_string(),
                        })
                    }
                    _ => None,
                }
            }
            "content_block_start" => {
                let block = data.get("content_block")?;
                let block_type = block.get("type").and_then(|t| t.as_str())?;
                if block_type == "tool_use" {
                    let id = block.get("id").and_then(|i| i.as_str()).map(String::from);
                    let name = block.get("name").and_then(|n| n.as_str()).map(String::from);
                    let index = data.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                    Some(SseDelta::ToolCallDelta {
                        index,
                        id,
                        name,
                        args_fragment: String::new(),
                    })
                } else {
                    None
                }
            }
            "message_delta" => {
                let usage = data.get("usage")?;
                let output = usage.get("output_tokens").and_then(|v| v.as_u64())?;
                Some(SseDelta::Usage {
                    input_tokens: 0,
                    output_tokens: output,
                })
            }
            "message_start" => {
                let message = data.get("message")?;
                let usage = message.get("usage")?;
                let input = usage.get("input_tokens").and_then(|v| v.as_u64())?;
                Some(SseDelta::Usage {
                    input_tokens: input,
                    output_tokens: 0,
                })
            }
            "message_stop" => Some(SseDelta::Done),
            _ => None,
        }
    }

    fn fold_sse_deltas(
        &self,
        deltas: Vec<SseDelta>,
        config: &AgentConfig,
    ) -> Result<TurnResult, AgentError> {
        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_builders: Vec<ToolBuilder> = Vec::new();
        let mut input_tokens: u64 = 0;
        let mut output_tokens: u64 = 0;

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
                        tool_builders.push(ToolBuilder::default());
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
                    input_tokens: i,
                    output_tokens: o,
                } => {
                    input_tokens += i;
                    output_tokens += o;
                }
                SseDelta::Done | SseDelta::StructuredValue(_) => {}
            }
        }

        let mut structured_value: Option<Value> = None;
        let mut tool_calls: Vec<HttpToolCall> = Vec::new();

        for builder in tool_builders {
            let name = builder.name.unwrap_or_default();
            let id = builder.id.unwrap_or_default();
            let input: Value = serde_json::from_str(&builder.args).unwrap_or(json!({}));

            if config.json_schema.is_some() && name == "structured_output" {
                structured_value = Some(input);
            } else {
                tool_calls.push(HttpToolCall { id, name, input });
            }
        }

        let text = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let is_final = tool_calls.is_empty();
        Ok(TurnResult {
            text,
            tool_calls,
            is_final,
            structured_value,
            usage: HttpUsage {
                input_tokens: Some(input_tokens),
                output_tokens: Some(output_tokens),
            },
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
            m if m == Model::SONNET => AnthropicModel::SONNET_5.to_string(),
            m if m == Model::OPUS => AnthropicModel::OPUS_5.to_string(),
            m if m == Model::HAIKU => AnthropicModel::HAIKU_4_5.to_string(),
            other => other.to_string(),
        }
    }
}

fn parse_anthropic_usage(body: &Value) -> HttpUsage {
    let usage = body.get("usage");
    HttpUsage {
        input_tokens: usage
            .and_then(|u| u.get("input_tokens"))
            .and_then(|v| v.as_u64()),
        output_tokens: usage
            .and_then(|u| u.get("output_tokens"))
            .and_then(|v| v.as_u64()),
    }
}

#[derive(Default)]
struct ToolBuilder {
    id: Option<String>,
    name: Option<String>,
    args: String,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn adapter() -> AnthropicApiAdapter {
        AnthropicApiAdapter::new("test-key".to_string())
    }

    #[test]
    fn build_request_basic() {
        let a = adapter();
        let config = AgentConfig::new("Hello");
        let body = a.build_request(&config).expect("build_request failed");

        assert_eq!(body["model"], AnthropicModel::SONNET_5);
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "Hello");
        assert!(body.get("system").is_none());
        assert!(body.get("tools").is_none());
    }

    #[test]
    fn build_request_with_system_prompt() {
        let a = adapter();
        let config = AgentConfig::new("Hi").system_prompt("Be concise");
        let body = a.build_request(&config).expect("build_request failed");

        assert_eq!(body["system"], "Be concise");
    }

    #[test]
    fn build_request_with_json_schema_uses_tool_forcing() {
        let a = adapter();
        let schema = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;
        let config = AgentConfig::new("Give x").output_schema_raw(schema).into();
        let body = a.build_request(&config).expect("build_request failed");

        assert_eq!(body["tools"][0]["name"], "structured_output");
        assert_eq!(body["tool_choice"]["type"], "tool");
        assert_eq!(body["tool_choice"]["name"], "structured_output");
    }

    #[test]
    fn parse_response_text() {
        let a = adapter();
        let body = json!({
            "content": [{"type": "text", "text": "Hello!"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5},
            "model": "claude-sonnet-4-6"
        });
        let config = AgentConfig::new("Hi");
        let result = a.parse_response(&body, &config).expect("parse failed");

        assert_eq!(result.text.as_deref(), Some("Hello!"));
        assert!(result.is_final);
        assert!(result.structured_value.is_none());
        assert_eq!(result.usage.input_tokens, Some(10));
        assert_eq!(result.usage.output_tokens, Some(5));
    }

    #[test]
    fn parse_response_structured_output() {
        let a = adapter();
        let body = json!({
            "content": [{"type": "tool_use", "id": "tu_1", "name": "structured_output", "input": {"x": 42}}],
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 15, "output_tokens": 10},
            "model": "claude-sonnet-4-6"
        });
        let schema = r#"{"type":"object"}"#;
        let config = AgentConfig::new("Give x").output_schema_raw(schema).into();
        let result = a.parse_response(&body, &config).expect("parse failed");

        assert_eq!(result.structured_value, Some(json!({"x": 42})));
        assert!(result.tool_calls.is_empty());
    }

    #[test]
    fn parse_sse_line_text_delta() {
        let a = adapter();
        let line =
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hi"}}"#;
        let delta = a.parse_sse_line(line).expect("parse_sse failed");
        assert!(matches!(delta, SseDelta::Text(t) if t == "Hi"));
    }

    #[test]
    fn parse_sse_line_message_stop() {
        let a = adapter();
        let line = r#"{"type":"message_stop"}"#;
        let delta = a.parse_sse_line(line).expect("parse_sse failed");
        assert!(matches!(delta, SseDelta::Done));
    }

    #[test]
    fn resolve_model_aliases() {
        let a = adapter();
        assert_eq!(a.resolve_model("sonnet"), AnthropicModel::SONNET_5);
        assert_eq!(a.resolve_model("opus"), AnthropicModel::OPUS_5);
        assert_eq!(a.resolve_model("haiku"), AnthropicModel::HAIKU_4_5);
        assert_eq!(a.resolve_model("claude-opus-4-6"), "claude-opus-4-6");
        assert_eq!(a.resolve_model("claude-opus-4-8"), "claude-opus-4-8");
        assert_eq!(a.resolve_model("claude-opus-5"), "claude-opus-5");
        assert_eq!(a.resolve_model("claude-fable-5"), "claude-fable-5");
    }

    #[test]
    fn auth_headers_format() {
        let a = adapter();
        let headers = a.auth_headers();
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "x-api-key" && v == "test-key")
        );
        assert!(
            headers
                .iter()
                .any(|(k, v)| k == "anthropic-version" && v == "2023-06-01")
        );
    }

    #[test]
    fn endpoint_url_constant() {
        let a = adapter();
        assert_eq!(
            a.endpoint_url("any"),
            "https://api.anthropic.com/v1/messages"
        );
    }
}
