//! Google Gemini API adapter implementation.

use serde_json::{Value, json};

use crate::error::AgentError;
use crate::operations::agent::Model;
use crate::pricing::{CostBreakdown, StaticPricing};
use crate::provider::AgentConfig;
use crate::providers::http::adapter::{HttpAgentAdapter, HttpToolCall, HttpUsage, TurnResult};
use crate::providers::http::sse::SseDelta;
use crate::providers::http::tools::ToolRegistry;
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

/// Name of the synthetic function carrying the structured answer when tools are attached.
const STRUCTURED_OUTPUT_FUNCTION: &str = "structured_output";

/// Adapter for the Google Gemini `generateContent` API.
///
/// Uses `responseSchema` in `generationConfig` for structured output, or a
/// synthetic `structured_output` function when registry tools are attached
/// (the schema cannot be combined with function calling on every model).
/// Authentication is via API key as a query parameter.
pub struct GeminiAdapter {
    api_key: String,
    base_url: String,
    default_model: String,
}

impl GeminiAdapter {
    /// Create a new adapter with the given API key.
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            base_url: "https://generativelanguage.googleapis.com".to_string(),
            default_model: GeminiModel::FLASH_3_5.to_string(),
        }
    }

    /// Override the API base URL (default `https://generativelanguage.googleapis.com`).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_core::providers::http::gemini::adapter::GeminiAdapter;
    ///
    /// let adapter = GeminiAdapter::new("key".to_string())
    ///     .with_base_url("http://127.0.0.1:8080".to_string());
    /// ```
    pub fn with_base_url(mut self, base_url: String) -> Self {
        self.base_url = base_url;
        self
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
            "{}/v1/models/{}:generateContent?key={}",
            self.base_url, model, self.api_key
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

        if let Some(system) = config.full_system_prompt() {
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
        let mut tool_structured: Option<Value> = None;

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
                    if config.json_schema.is_some() && name == STRUCTURED_OUTPUT_FUNCTION {
                        tool_structured = Some(args);
                        continue;
                    }
                    // Gemini has no call id (a `functionResponse` is matched by
                    // name), and Gemini 3 rejects a follow-up request whose
                    // `functionCall` part lacks the signature it returned.
                    // Storing the signature in `id` carries it to
                    // `assistant_tool_message` without a new field on the
                    // public `HttpToolCall` struct.
                    let id = part
                        .get("thoughtSignature")
                        .and_then(|s| s.as_str())
                        .map(String::from)
                        .unwrap_or_else(|| name.clone());
                    tool_calls.push(HttpToolCall {
                        id,
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

        let text = if text_parts.is_empty() {
            None
        } else {
            Some(text_parts.join(""))
        };

        let from_tool = tool_structured.is_some();
        // The structured answer ends the conversation, registry calls of the
        // same turn are dropped.
        if from_tool {
            tool_calls.clear();
        }

        // A function-call turn also reports `STOP`, so it is final only
        // without tool calls.
        let finished = finish_reason == "STOP" || finish_reason == "MAX_TOKENS";
        let is_final = from_tool || (tool_calls.is_empty() && finished);

        let structured_value = if from_tool {
            tool_structured
        } else if config.json_schema.is_some() {
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
            let (uncached, cached) = split_prompt_tokens(usage);
            let input = uncached.unwrap_or(0);
            let cache_read = cached.unwrap_or(0);
            let output = usage
                .get("candidatesTokenCount")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            if input > 0 || cache_read > 0 || output > 0 {
                return Some(SseDelta::Usage {
                    input_tokens: input,
                    cache_read_input_tokens: cache_read,
                    cache_creation_input_tokens: 0,
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
                    cache_read_input_tokens,
                    output_tokens,
                    ..
                } => {
                    usage.input_tokens = Some(input_tokens);
                    usage.cache_read_input_tokens =
                        (cache_read_input_tokens > 0).then_some(cache_read_input_tokens);
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

    fn history_field(&self) -> &'static str {
        "contents"
    }

    fn attach_tools(&self, request_body: &mut Value, registry: &ToolRegistry) {
        let mut declarations: Vec<Value> = registry
            .to_function_declarations()
            .into_iter()
            .map(|mut declaration| {
                if let Some(parameters) = declaration.get("parameters") {
                    declaration["parameters"] = adapt_schema_for_gemini(parameters);
                }
                declaration
            })
            .collect();

        // Gemini 2.5 models reject a JSON response mime type combined with
        // function calling. The schema moves into a synthetic function and mode
        // `ANY` forces a function call every turn, either a registry function
        // (the loop continues) or `structured_output` (the answer): same
        // contract as Anthropic's `tool_choice: any`.
        let response_schema = request_body
            .get("generationConfig")
            .and_then(|g| g.get("responseSchema"))
            .cloned();
        if let Some(schema) = response_schema {
            declarations.insert(
                0,
                json!({
                    "name": STRUCTURED_OUTPUT_FUNCTION,
                    "description": "Output the result in the requested JSON schema.",
                    "parameters": schema
                }),
            );
            let mut generation_config_empty = false;
            if let Some(generation_config) = request_body
                .get_mut("generationConfig")
                .and_then(|g| g.as_object_mut())
            {
                generation_config.remove("responseMimeType");
                generation_config.remove("responseSchema");
                generation_config_empty = generation_config.is_empty();
            }
            if generation_config_empty && let Some(body) = request_body.as_object_mut() {
                body.remove("generationConfig");
            }
            request_body["toolConfig"] = json!({"functionCallingConfig": {"mode": "ANY"}});
        }

        request_body["tools"] = json!([{ "functionDeclarations": declarations }]);
    }

    fn assistant_tool_message(&self, turn: &TurnResult) -> Value {
        let mut parts: Vec<Value> = Vec::new();
        if let Some(text) = turn.text.as_deref().filter(|t| !t.is_empty()) {
            parts.push(json!({"text": text}));
        }
        parts.extend(turn.tool_calls.iter().map(|tc| {
            let mut part = json!({"functionCall": {"name": tc.name, "args": tc.input}});
            // An id different from the name is the signature of the call.
            if tc.id != tc.name {
                part["thoughtSignature"] = Value::String(tc.id.clone());
            }
            part
        }));
        json!({"role": "model", "parts": parts})
    }

    fn tool_result_messages(
        &self,
        calls: &[HttpToolCall],
        results: &[(String, bool)],
    ) -> Vec<Value> {
        let parts: Vec<Value> = calls
            .iter()
            .zip(results)
            .map(|(call, (content, is_error))| {
                let response = if *is_error {
                    json!({"error": content})
                } else {
                    json!({"output": content})
                };
                json!({"functionResponse": {"name": call.name, "response": response}})
            })
            .collect();
        vec![json!({"role": "user", "parts": parts})]
    }

    fn compute_cost(&self, model: &str, usage: &HttpUsage) -> Option<f64> {
        let pricing = StaticPricing::new();
        let bd = CostBreakdown::compute_with_cache(
            &pricing,
            model,
            usage.input_tokens.unwrap_or(0),
            usage.cache_read_input_tokens.unwrap_or(0),
            usage.cache_creation_input_tokens.unwrap_or(0),
            usage.output_tokens.unwrap_or(0),
        );
        Some(bd.total_usd)
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

/// Split Gemini `promptTokenCount` into `(uncached, cached)`.
///
/// Gemini's `promptTokenCount` includes `cachedContentTokenCount`, so the
/// cached part is subtracted (saturating at 0) to keep `input_tokens` uncached.
fn split_prompt_tokens(usage: &Value) -> (Option<u64>, Option<u64>) {
    let prompt = usage.get("promptTokenCount").and_then(|v| v.as_u64());
    let cached = usage
        .get("cachedContentTokenCount")
        .and_then(|v| v.as_u64());
    (
        prompt.map(|p| p.saturating_sub(cached.unwrap_or(0))),
        cached,
    )
}

fn parse_gemini_usage(body: &Value) -> HttpUsage {
    let usage = body.get("usageMetadata");
    let (input_tokens, cache_read_input_tokens) = match usage {
        Some(u) => split_prompt_tokens(u),
        None => (None, None),
    };
    HttpUsage {
        input_tokens,
        cache_read_input_tokens,
        cache_creation_input_tokens: None,
        output_tokens: usage
            .and_then(|u| u.get("candidatesTokenCount"))
            .and_then(|v| v.as_u64()),
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;

    use serde_json::json;

    use super::*;
    use crate::providers::http::tools::{Tool, ToolError, ToolOutput};

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
        let config = AgentConfig::new("Hi").system_prompt("Be brief");
        let body = a.build_request(&config).expect("build_request failed");

        assert_eq!(body["system_instruction"]["parts"][0]["text"], "Be brief");
    }

    #[test]
    fn build_request_with_json_schema() {
        let a = adapter();
        let schema = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;
        let config = AgentConfig::new("Give x").output_schema_raw(schema).into();
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
    fn parse_response_cache_read_cached_content_token_count() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "Hello!"}], "role": "model"}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 1000, "candidatesTokenCount": 5, "cachedContentTokenCount": 600}
        });
        let config = AgentConfig::new("Hi");
        let result = a.parse_response(&body, &config).expect("parse failed");

        assert_eq!(result.usage.input_tokens, Some(400));
        assert_eq!(result.usage.cache_read_input_tokens, Some(600));
        assert!(result.usage.cache_creation_input_tokens.is_none());
        assert_eq!(result.usage.output_tokens, Some(5));
    }

    #[test]
    fn parse_response_cache_read_absent_is_none() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "Hello!"}], "role": "model"}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 1000, "candidatesTokenCount": 5}
        });
        let config = AgentConfig::new("Hi");
        let result = a.parse_response(&body, &config).expect("parse failed");

        assert_eq!(result.usage.input_tokens, Some(1000));
        assert!(result.usage.cache_read_input_tokens.is_none());
    }

    #[test]
    fn parse_response_cache_read_larger_than_prompt_saturates() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "Hello!"}], "role": "model"}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 100, "candidatesTokenCount": 5, "cachedContentTokenCount": 300}
        });
        let config = AgentConfig::new("Hi");
        let result = a.parse_response(&body, &config).expect("parse failed");

        assert_eq!(result.usage.input_tokens, Some(0));
        assert_eq!(result.usage.cache_read_input_tokens, Some(300));
    }

    #[test]
    fn parse_sse_line_cache_read_usage() {
        let a = adapter();
        let line = r#"{"candidates":[{"content":{"parts":[],"role":"model"}}],"usageMetadata":{"promptTokenCount":1000,"candidatesTokenCount":5,"cachedContentTokenCount":600}}"#;
        let delta = a.parse_sse_line(line).expect("parse_sse failed");
        assert!(matches!(
            delta,
            SseDelta::Usage {
                input_tokens: 400,
                cache_read_input_tokens: 600,
                cache_creation_input_tokens: 0,
                output_tokens: 5
            }
        ));

        let result = a
            .fold_sse_deltas(vec![delta], &AgentConfig::new("Hi"))
            .expect("fold failed");
        assert_eq!(result.usage.input_tokens, Some(400));
        assert_eq!(result.usage.cache_read_input_tokens, Some(600));
    }

    #[test]
    fn parse_response_structured() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [{"text": "{\"x\":42}"}], "role": "model"}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5}
        });
        let schema = r#"{"type":"object"}"#;
        let config = AgentConfig::new("Give x").output_schema_raw(schema).into();
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

    struct LookupTool;

    impl Tool for LookupTool {
        fn name(&self) -> &str {
            "lookup"
        }
        fn description(&self) -> &str {
            "Looks something up"
        }
        fn parameters_schema(&self) -> Value {
            json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {"q": {"type": "string"}}
            })
        }
        fn execute(
            &self,
            _input: Value,
        ) -> Pin<Box<dyn Future<Output = Result<ToolOutput, ToolError>> + Send + '_>> {
            Box::pin(async { Ok(ToolOutput::success("found")) })
        }
    }

    fn registry() -> ToolRegistry {
        ToolRegistry::new().register(LookupTool)
    }

    fn declaration_names(body: &Value) -> Vec<&str> {
        body["tools"][0]["functionDeclarations"]
            .as_array()
            .expect("declarations")
            .iter()
            .filter_map(|d| d["name"].as_str())
            .collect()
    }

    #[test]
    fn attach_tools_uses_function_declarations() {
        let a = adapter();
        let mut body = a
            .build_request(&AgentConfig::new("Hi"))
            .expect("build_request failed");
        a.attach_tools(&mut body, &registry());

        assert_eq!(declaration_names(&body), vec!["lookup"]);
        let parameters = &body["tools"][0]["functionDeclarations"][0]["parameters"];
        assert_eq!(parameters["type"], "OBJECT");
        assert_eq!(parameters["properties"]["q"]["type"], "STRING");
        assert!(parameters.get("additionalProperties").is_none());
        assert!(body.get("toolConfig").is_none());
        assert!(body.get("generationConfig").is_none());
    }

    #[test]
    fn attach_tools_moves_response_schema_into_structured_output_function() {
        let a = adapter();
        let schema = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;
        let config = AgentConfig::new("Give x").output_schema_raw(schema).into();
        let mut body = a.build_request(&config).expect("build_request failed");
        a.attach_tools(&mut body, &registry());

        assert_eq!(
            declaration_names(&body),
            vec!["structured_output", "lookup"]
        );
        assert_eq!(
            body["tools"][0]["functionDeclarations"][0]["parameters"]["type"],
            "OBJECT"
        );
        assert!(body.get("generationConfig").is_none());
        assert_eq!(body["toolConfig"]["functionCallingConfig"]["mode"], "ANY");
    }

    #[test]
    fn parse_response_function_call_with_stop_is_not_final() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [{"functionCall": {"name": "lookup", "args": {"q": "a"}}}], "role": "model"}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5}
        });
        let result = a
            .parse_response(&body, &AgentConfig::new("Hi"))
            .expect("parse failed");

        assert!(!result.is_final);
        assert_eq!(result.tool_calls.len(), 1);
        assert_eq!(result.tool_calls[0].name, "lookup");
        assert_eq!(result.tool_calls[0].id, "lookup");
    }

    #[test]
    fn parse_response_structured_output_function_is_final() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [
                {"functionCall": {"name": "lookup", "args": {"q": "a"}}},
                {"functionCall": {"name": "structured_output", "args": {"x": 7}}}
            ], "role": "model"}, "finishReason": "STOP"}],
            "usageMetadata": {"promptTokenCount": 10, "candidatesTokenCount": 5}
        });
        let schema = r#"{"type":"object"}"#;
        let config = AgentConfig::new("Give x").output_schema_raw(schema).into();
        let result = a.parse_response(&body, &config).expect("parse failed");

        assert!(result.is_final);
        assert!(result.tool_calls.is_empty());
        assert_eq!(result.structured_value, Some(json!({"x": 7})));
    }

    #[test]
    fn parse_response_keeps_thought_signature_in_call_id() {
        let a = adapter();
        let body = json!({
            "candidates": [{"content": {"parts": [
                {"functionCall": {"name": "lookup", "args": {}}, "thoughtSignature": "sig-1"},
                {"functionCall": {"name": "search", "args": {}}}
            ], "role": "model"}, "finishReason": "STOP"}]
        });
        let result = a
            .parse_response(&body, &AgentConfig::new("Hi"))
            .expect("parse failed");

        assert_eq!(result.tool_calls[0].id, "sig-1");
        assert_eq!(result.tool_calls[1].id, "search");
    }

    #[test]
    fn assistant_tool_message_echoes_thought_signature() {
        let a = adapter();
        let turn = TurnResult {
            text: Some("checking".to_string()),
            tool_calls: vec![
                HttpToolCall {
                    id: "sig-1".to_string(),
                    name: "lookup".to_string(),
                    input: json!({"q": "a"}),
                },
                HttpToolCall {
                    id: "search".to_string(),
                    name: "search".to_string(),
                    input: json!({}),
                },
            ],
            is_final: false,
            structured_value: None,
            usage: HttpUsage::default(),
            model: None,
        };
        let msg = a.assistant_tool_message(&turn);

        assert_eq!(msg["role"], "model");
        assert_eq!(msg["parts"][0]["text"], "checking");
        assert_eq!(msg["parts"][1]["functionCall"]["name"], "lookup");
        assert_eq!(msg["parts"][1]["functionCall"]["args"], json!({"q": "a"}));
        assert_eq!(msg["parts"][1]["thoughtSignature"], "sig-1");
        assert!(msg["parts"][2].get("thoughtSignature").is_none());
    }

    #[test]
    fn tool_result_messages_use_function_response() {
        let a = adapter();
        let calls = vec![
            HttpToolCall {
                id: "lookup".to_string(),
                name: "lookup".to_string(),
                input: json!({}),
            },
            HttpToolCall {
                id: "search".to_string(),
                name: "search".to_string(),
                input: json!({}),
            },
        ];
        let results = vec![("found".to_string(), false), ("boom".to_string(), true)];
        let messages = a.tool_result_messages(&calls, &results);

        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        let ok = &messages[0]["parts"][0]["functionResponse"];
        assert_eq!(ok["name"], "lookup");
        assert_eq!(ok["response"]["output"], "found");
        let err = &messages[0]["parts"][1]["functionResponse"];
        assert_eq!(err["response"]["error"], "boom");
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
