//! JSON Schema transformations for Claude CLI compatibility.
//!
//! The Claude CLI `--json-schema` flag has known compatibility issues with
//! certain JSON Schema features. This module normalizes schemas before
//! sending them to the CLI to maximize structured output reliability.
//!
//! # Transformations applied
//!
//! 1. **`additionalProperties: false`** - added to every object that lacks it
//!    (required by Anthropic's constrained decoding).
//! 2. **`$ref` / `$defs` inlining** - JSON Schema references are resolved
//!    and inlined, since the CLI may not support `$ref`.
//! 3. **Meta-field removal** - `$schema`, `title`, and `default` are stripped
//!    (not used by the CLI and may cause issues).
//!
//! # Examples
//!
//! ```
//! use ironflow_core::schema_transform::transform_schema;
//!
//! let schema = r#"{"type":"object","properties":{"name":{"type":"string"}}}"#;
//! let transformed = transform_schema(schema);
//! assert!(transformed.contains("additionalProperties"));
//! ```

use serde_json::{Map, Value};
use tracing::warn;

/// Transform a JSON Schema string for Claude CLI compatibility.
///
/// Parses the schema, applies all transformations, and re-serializes.
/// Returns the original string unchanged if parsing fails.
///
/// # Examples
///
/// ```
/// use ironflow_core::schema_transform::transform_schema;
///
/// let schema = r#"{"type":"object","properties":{"x":{"type":"integer"}}}"#;
/// let result = transform_schema(schema);
/// assert!(result.contains(r#""additionalProperties":false"#));
/// ```
pub fn transform_schema(schema: &str) -> String {
    let mut value: Value = match serde_json::from_str(schema) {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "failed to parse JSON schema for transformation, using original");
            return schema.to_string();
        }
    };

    let defs = extract_defs(&value);
    remove_meta_fields(&mut value);
    inline_refs(&mut value, &defs);
    add_additional_properties_false(&mut value);

    serde_json::to_string(&value).unwrap_or_else(|_| schema.to_string())
}

/// Extract `$defs` (or `definitions`) from the root schema for ref inlining.
fn extract_defs(schema: &Value) -> Map<String, Value> {
    schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default()
}

/// Remove `$schema`, `title`, `default`, `$defs`, and `definitions` from all levels.
fn remove_meta_fields(value: &mut Value) {
    if let Some(obj) = value.as_object_mut() {
        obj.remove("$schema");
        obj.remove("title");
        obj.remove("default");
        obj.remove("$defs");
        obj.remove("definitions");

        for (_, v) in obj.iter_mut() {
            remove_meta_fields(v);
        }
    } else if let Some(arr) = value.as_array_mut() {
        for item in arr.iter_mut() {
            remove_meta_fields(item);
        }
    }
}

/// Recursively replace `{"$ref": "#/$defs/Foo"}` with the inlined definition.
fn inline_refs(value: &mut Value, defs: &Map<String, Value>) {
    match value {
        Value::Object(obj) => {
            if let Some(ref_val) = obj.get("$ref").and_then(|v| v.as_str()).map(String::from)
                && let Some(resolved) = resolve_ref(&ref_val, defs)
            {
                let mut resolved = resolved.clone();
                inline_refs(&mut resolved, defs);
                *value = resolved;
                return;
            }

            for (_, v) in obj.iter_mut() {
                inline_refs(v, defs);
            }
        }
        Value::Array(arr) => {
            for item in arr.iter_mut() {
                inline_refs(item, defs);
            }
        }
        _ => {}
    }
}

/// Resolve a `$ref` path like `#/$defs/MyType` or `#/definitions/MyType`.
fn resolve_ref<'a>(ref_path: &str, defs: &'a Map<String, Value>) -> Option<&'a Value> {
    let name = ref_path
        .strip_prefix("#/$defs/")
        .or_else(|| ref_path.strip_prefix("#/definitions/"))?;
    defs.get(name)
}

/// Add `"additionalProperties": false` to every object schema that lacks it.
fn add_additional_properties_false(value: &mut Value) {
    if let Some(obj) = value.as_object_mut() {
        let is_object_schema = obj.get("type").and_then(|t| t.as_str()) == Some("object");

        if is_object_schema && !obj.contains_key("additionalProperties") {
            obj.insert("additionalProperties".to_string(), Value::Bool(false));
        }

        for (_, v) in obj.iter_mut() {
            add_additional_properties_false(v);
        }
    } else if let Some(arr) = value.as_array_mut() {
        for item in arr.iter_mut() {
            add_additional_properties_false(item);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn adds_additional_properties_to_simple_object() {
        let schema = r#"{"type":"object","properties":{"name":{"type":"string"}}}"#;
        let result = transform_schema(schema);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["additionalProperties"], json!(false));
    }

    #[test]
    fn adds_additional_properties_to_nested_objects() {
        let schema = json!({
            "type": "object",
            "properties": {
                "address": {
                    "type": "object",
                    "properties": {
                        "city": {"type": "string"}
                    }
                }
            }
        });
        let result = transform_schema(&schema.to_string());
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["additionalProperties"], json!(false));
        assert_eq!(
            parsed["properties"]["address"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn preserves_existing_additional_properties() {
        let schema = json!({
            "type": "object",
            "properties": {"x": {"type": "integer"}},
            "additionalProperties": true
        });
        let result = transform_schema(&schema.to_string());
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed["additionalProperties"], json!(true));
    }

    #[test]
    fn inlines_defs_refs() {
        let schema = json!({
            "type": "object",
            "properties": {
                "item": {"$ref": "#/$defs/Item"}
            },
            "$defs": {
                "Item": {
                    "type": "object",
                    "properties": {
                        "name": {"type": "string"}
                    }
                }
            }
        });
        let result = transform_schema(&schema.to_string());
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert!(parsed.get("$defs").is_none());
        assert_eq!(parsed["properties"]["item"]["type"], json!("object"));
        assert_eq!(
            parsed["properties"]["item"]["properties"]["name"]["type"],
            json!("string")
        );
        assert_eq!(
            parsed["properties"]["item"]["additionalProperties"],
            json!(false)
        );
    }

    #[test]
    fn inlines_definitions_refs() {
        let schema = json!({
            "type": "object",
            "properties": {
                "item": {"$ref": "#/definitions/Item"}
            },
            "definitions": {
                "Item": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer"}
                    }
                }
            }
        });
        let result = transform_schema(&schema.to_string());
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert!(parsed.get("definitions").is_none());
        assert_eq!(parsed["properties"]["item"]["type"], json!("object"));
    }

    #[test]
    fn removes_meta_fields() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "title": "MySchema",
            "type": "object",
            "properties": {
                "score": {
                    "type": "integer",
                    "title": "The Score",
                    "default": 0
                }
            }
        });
        let result = transform_schema(&schema.to_string());
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert!(parsed.get("$schema").is_none());
        assert!(parsed.get("title").is_none());
        assert!(parsed["properties"]["score"].get("title").is_none());
        assert!(parsed["properties"]["score"].get("default").is_none());
    }

    #[test]
    fn idempotent_on_already_transformed_schema() {
        let schema = json!({
            "type": "object",
            "properties": {
                "x": {"type": "integer"}
            },
            "additionalProperties": false
        });
        let input = schema.to_string();
        let first = transform_schema(&input);
        let second = transform_schema(&first);
        assert_eq!(first, second);
    }

    #[test]
    fn handles_invalid_json_gracefully() {
        let bad = "not valid json{";
        let result = transform_schema(bad);
        assert_eq!(result, bad);
    }

    #[test]
    fn handles_non_object_schema() {
        let schema = r#"{"type":"string"}"#;
        let result = transform_schema(schema);
        let parsed: Value = serde_json::from_str(&result).unwrap();
        assert!(parsed.get("additionalProperties").is_none());
    }

    #[test]
    fn inlines_nested_refs() {
        let schema = json!({
            "type": "object",
            "properties": {
                "items": {
                    "type": "array",
                    "items": {"$ref": "#/$defs/Item"}
                }
            },
            "$defs": {
                "Item": {
                    "type": "object",
                    "properties": {
                        "nested": {"$ref": "#/$defs/Nested"}
                    }
                },
                "Nested": {
                    "type": "object",
                    "properties": {
                        "value": {"type": "string"}
                    }
                }
            }
        });
        let result = transform_schema(&schema.to_string());
        let parsed: Value = serde_json::from_str(&result).unwrap();

        let item = &parsed["properties"]["items"]["items"];
        assert_eq!(item["type"], json!("object"));
        assert_eq!(item["additionalProperties"], json!(false));

        let nested = &item["properties"]["nested"];
        assert_eq!(nested["type"], json!("object"));
        assert_eq!(nested["additionalProperties"], json!(false));
        assert_eq!(nested["properties"]["value"]["type"], json!("string"));
    }

    #[test]
    fn handles_schemars_generated_schema() {
        let schema = json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "title": "Review",
            "type": "object",
            "required": ["score", "summary"],
            "properties": {
                "score": {"type": "integer", "default": 5},
                "summary": {"type": "string"}
            }
        });
        let result = transform_schema(&schema.to_string());
        let parsed: Value = serde_json::from_str(&result).unwrap();

        assert!(parsed.get("$schema").is_none());
        assert!(parsed.get("title").is_none());
        assert!(parsed["properties"]["score"].get("default").is_none());
        assert_eq!(parsed["additionalProperties"], json!(false));
        assert_eq!(parsed["required"], json!(["score", "summary"]));
    }
}
