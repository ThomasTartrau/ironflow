//! Google Gemini API adapter implementation.

use serde_json::{json, Value};

use crate::error::AgentError;
use crate::operations::agent::Model;
use crate::provider::AgentConfig;
use crate::providers::http::adapter::{HttpAgentAdapter, HttpToolCall, HttpUsage, TurnResult};
use crate::providers::http::cost::GEMINI_COSTS;
use crate::providers::http::sse::SseDelta;
use crate::schema_transform::transform_schema;

/// Known Google Gemini model identifiers.
pub struct GeminiModel;

impl GeminiModel {
    /// Gemini 3.5 Flash - most intelligent for agentic and coding tasks (stable).
    pub const FLASH_3_5: &str = "gemini-3.5-flash";
    /// Gemini 3.1 Flash Lite - cost-efficient performance (stable).
    pub const FLASH_LITE_3_1: &str = "gemini-3.1-flash-lite";
    /// Gemini 2.5 Pro - advanced model for complex tasks (1M context, stable).
    pub const PRO_2_5: &str = "gemini-2.5-pro";
    /// Gemini 2.5 Flash - best price-performance for reasoning (1M context, stable).
    pub const FLASH_2_5: &str = "gemini-2.5-flash";
    /// Gemini 2.5 Flash Lite - fastest and most budget-friendly (1M context, stable).
    pub const FLASH_LITE_2_5: &str = "gemini-2.5-flash-lite";
}

/// Adapter for the Google Gemini `generateContent` API.
///
/// Uses `responseSchema` in `generationConfig` for structured output.
/// Authentication is via API key as a query parameter.
pub struct GeminiAdapter {
    api_key: String,
    default_model: String,
}

impl GeminiAdapter {
    /// Create a new adapter with the given API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            default_model: GeminiModel::FLASH_3_5.to_string(),
        }
    }

    /// Override the default model.
    pub fn with_default_model(mut self, model: String) -> Self {
        self.default_model = model;
        self
    }
}

impl HttpAgentAdapter for GeminiAdapter {
    fn provider_name(&self) -> &'static str {
        "gemini"
    }

    fn endpoint_url(&self, model: &str) -> String {
        format!(
            "https://generativelanguage.googleapis.com/v1/models/{}:generateContent?key={}",
            model, self.api_key
        )
    }

    fn auth_headers(&self) -> Vec<(String, String)> {
        vec![("content-type".to_string(), "application/json".to_string())]
    }

    fn build_request(&self, config: &AgentConfig) -> Result<Value, AgentError> {
        let mut contents: Vec<Value> = Vec::new();

        contents.push(json!({
            "role": "user",
            "parts": [{ "text": config.prompt }]
        }));

        let mut body = json!({ "contents": contents });

        if let Some(ref system) = config.system_prompt {
            body["system_instruction"] = json!({
                "parts": [{ "text": system }]
            });
        }

        if let Some(ref schema_str) = config.json_schema {
            let transformed = transform_schema(schema_str);
            let schema_value: Value = serde_json::from_str(&transformed).unwrap_or(json!({}));
            let gemini_schema = adapt_schema_for_gemini(&schema_value);

            body["generationConfig"] = json!({
                "responseMimeType": "application/json",
                "responseSchema": gemini_schema
            });
        }

        Ok(body)
    }

    fn parse_response(&self, body: &Value, config: &AgentConfig) -> Result<TurnResult, AgentError> {
        let candidate = body.get("candidates").and_then(|c| c.get(0));

        let parts = candidate
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.as_array());

        let mut text_parts: Vec<String> = Vec::new();
        let mut tool_calls: Vec<HttpToolCall> = Vec::new();

        if let Some(parts) = parts {
            for part in parts {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    text_parts.push(text.to_string());
                }
                if let Some(fc) = part.get("functionCall") {
                    let name = fc
                        .get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string();
                    let args = fc.get("args").cloned().unwrap_or(json!({}));
                    tool_calls.push(HttpToolCall {
                        id: name.clone(),
                        name,
                        input: args,
                    });
                }
            }
        }

        let finish_reason = candidate
            .and_then(|c| c.get("finishReason"))
            .and_then(|f| f.as_str())
            .unwrap_or("STOP");

        let is_final = finish_reason == "STOP" || finish_reason == "MAX_TOKENS";

        let text = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let structured_value = if config.json_schema.is_some() {
            text.as_deref()
                .and_then(|t| serde_json::from_str::<Value>(t).ok())
        } else {
            None
        };

        let usage = parse_gemini_usage(body);
        let model = body
            .get("modelVersion")
            .and_then(|m| m.as_str())
            .map(String::from);

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
        let data: Value = serde_json::from_str(line).ok()?;

        let candidates = data.get("candidates")?.as_array()?;
        let candidate = candidates.first()?;
        let parts = candidate.get("content")?.get("parts")?.as_array()?;

        for part in parts {
            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                return Some(SseDelta::Text(text.to_string()));
            }
            if let Some(fc) = part.get("functionCall") {
                let name = fc.get("name").and_then(|n| n.as_str()).map(String::from);
                let args = fc.get("args").map(|a| a.to_string()).unwrap_or_default();
                return Some(SseDelta::ToolCallDelta {
                    index: 0,
                    id: name.clone(),
                    name,
                    args_fragment: args,
                });
            }
        }

        if let Some(usage) = data.get("usageMetadata") {
            let input = usage
                .get("promptTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            let output = usage
                .get("candidatesTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if input > 0 || output > 0 {
                return Some(SseDelta::Usage {
                    input_tokens: input,
                    output_tokens: output,
                });
            }
        }

        None
    }

    fn fold_sse_deltas(
        &self,
        deltas: Vec<SseDelta>,
        config: &AgentConfig,
    ) -> Result<TurnResult, AgentError> {
        let mut text_parts: Vec<String> = Vec::new();
        let mut usage = HttpUsage::default();

        for delta in deltas {
            match delta {
                SseDelta::Text(t) => text_parts.push(t),
                SseDelta::Usage {
                    input_tokens,
                    output_tokens,
                } => {
                    usage.input_tokens = Some(input_tokens);
                    usage.output_tokens = Some(output_tokens);
                }
                _ => {}
            }
        }

        let text = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let structured_value = if config.json_schema.is_some() {
            text.as_deref()
                .and_then(|t| serde_json::from_str::<Value>(t).ok())
        } else {
            None
        };

        Ok(TurnResult {
            text: if structured_value.is_some() {
                None
            } else {
                text
            },
            tool_calls: Vec::new(),
            is_final: true,
            structured_value,
            usage,
            model: None,
        })
    }

    fn compute_cost(&self, model: &str, input_tokens: u64, output_tokens: u64) -> Option<f64> {
        GEMINI_COSTS.compute(model, input_tokens, output_tokens)
    }

    fn resolve_model(&self, model: &str) -> String {
        match model {
            m if m == Model::SONNET => GeminiModel::FLASH_3_5.to_string(),
            m if m == Model::OPUS => GeminiModel::PRO_2_5.to_string(),
            m if m == Model::HAIKU => GeminiModel::FLASH_LITE_3_1.to_string(),
            other => other.to_string(),
        }
    }
}

/// Adapt a JSON Schema value for Gemini's OpenAPI-style schema format.
///
/// Gemini expects uppercase type names and does not support all JSON Schema features.
fn adapt_schema_for_gemini(schema: &Value) -> Value {
    match schema {
        Value::Object(obj) => {
            let mut result = serde_json::Map::new();
            for (key, value) in obj {
                if key == "type" {
                    if let Some(t) = value.as_str() {
                        result.insert(key.clone(), Value::String(t.to_uppercase()));
                    } else {
                        result.insert(key.clone(), adapt_schema_for_gemini(value));
                    }
                } else if key == "additionalProperties" {
                    // Gemini does not support additionalProperties
                    continue;
                } else {
                    result.insert(key.clone(), adapt_schema_for_gemini(value));
                }
            }
            Value::Object(result)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(adapt_schema_for_gemini).collect()),
        other => other.clone(),
    }
}

fn parse_gemini_usage(body: &Value) -> HttpUsage {
    let usage = body.get("usageMetadata");
    HttpUsage {
        input_tokens: usage
            .and_then(|u| u.get("promptTokenCount"))
            .and_then(|v| v.as_u64()),
        output_tokens: usage
            .and_then(|u| u.get("candidatesTokenCount"))
            .and_then(|v| v.as_u64()),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn adapter() -> GeminiAdapter {
        GeminiAdapter::new("test-key".to_string())
    }

    #[test]
    fn build_request_basic() {
        let a = adapter();
        let config = AgentConfig::new("Hello");
        let body = a.build_request(&config).expect("build_request failed");

        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "Hello");
        assert!(body.get("system_instruction").is_none());
        assert!(body.get("generationConfig").is_none());
    }

    #[test]
    fn build_request_with_system_prompt() {
        let a = adapter();
        let config = AgentConfig::new("Hi")
            .system_prompt("Be brief");
        let body = a.build_request(&config).expect("build_request failed");

        assert_eq!(body["system_instruction"]["parts"][0]["text"], "Be brief");
    }

    #[test]
    fn build_request_with_json_schema() {
        let a = adapter();
        let schema = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;
        let config = AgentConfig::new("Give x")
            .output_schema_raw(schema).into();
        let body = a.build_request(&config).expect("build_request failed");

        assert_eq!(
            body["generationConfig"]["responseMimeType"],
            "application/json"
        );
        assert!(body["generationConfig"]["responseSchema"].is_object());
    }

    #[test]
    fn parse_response_text() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "Hello!"}], "role": "model"}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5},
            "modelVersion": "gemini-3.5-flash"
        });
        let config = AgentConfig::new("Hi");
        let result = a.parse_response(&body, &config).expect("parse failed");

        assert_eq!(result.text.as_deref(), Some("Hello!"));
        assert!(result.is_final);
        assert_eq!(result.usage.input_tokens, Some(10));
        assert_eq!(result.usage.output_tokens, Some(5));
        assert_eq!(result.model.as_deref(), Some("gemini-3.5-flash"));
    }

    #[test]
    fn parse_response_structured() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "{\"x\":42}"}], "role": "model"}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5}
        });
        let schema = r#"{"type":"object"}"#;
        let config = AgentConfig::new("Give x")
            .output_schema_raw(schema).into();
        let result = a.parse_response(&body, &config).expect("parse failed");

        assert!(result.text.is_none());
        assert_eq!(result.structured_value, Some(json!({"x": 42})));
    }

    #[test]
    fn resolve_model_aliases() {
        let a = adapter();
        assert_eq!(a.resolve_model("sonnet"), GeminiModel::FLASH_3_5);
        assert_eq!(a.resolve_model("opus"), GeminiModel::PRO_2_5);
        assert_eq!(a.resolve_model("haiku"), GeminiModel::FLASH_LITE_3_1);
        assert_eq!(a.resolve_model("gemini-2.5-pro"), "gemini-2.5-pro");
    }

    #[test]
    fn endpoint_url_includes_model_and_key() {
        let a = adapter();
        let url = a.endpoint_url("gemini-3.5-flash");
        assert!(url.contains("gemini-3.5-flash"));
        assert!(url.contains("key=test-key"));
        assert!(url.starts_with("https://generativelanguage.googleapis.com/v1/"));
    }

    #[test]
    fn adapt_schema_uppercases_types() {
        let schema = json!({"type": "object", "properties": {"x": {"type": "string"}}});
        let adapted = adapt_schema_for_gemini(&schema);
        assert_eq!(adapted["type"], "OBJECT");
        assert_eq!(adapted["properties"]["x"]["type"], "STRING");
    }

    #[test]
    fn adapt_schema_removes_additional_properties() {
        let schema = json!({"type": "object", "additionalProperties": false});
        let adapted = adapt_schema_for_gemini(&schema);
        assert!(adapted.get("additionalProperties").is_none());
    }
}
