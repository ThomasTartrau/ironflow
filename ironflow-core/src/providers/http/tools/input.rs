//! Typed tool arguments: one Rust type gives both the JSON Schema sent to the
//! model and the parsing of the arguments the model sends back, so the two
//! cannot drift apart.

use schemars::generate::SchemaSettings;
use schemars::transform::RecursiveTransform;
use schemars::{JsonSchema, Schema};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::tool_trait::ToolOutput;

/// JSON Schema of the arguments type `T`, ready for
/// [`Tool::parameters_schema`](super::Tool::parameters_schema).
///
/// Field doc comments become the property descriptions. Subschemas are
/// inlined (no `$ref`), and the root carries no `$schema`, `title` nor
/// `description`: the tool description already says what the tool does.
/// An `Option<U>` field is advertised as an optional `U`, not as
/// `["U", "null"]`: the model should omit it, and Gemini rejects a `type`
/// array. [`parse_input`] still accepts `null` for it.
pub(crate) fn input_schema<T: JsonSchema>() -> Value {
    let mut schema = SchemaSettings::draft2020_12()
        .with(|settings| {
            settings.meta_schema = None;
            settings.inline_subschemas = true;
        })
        .with_transform(RecursiveTransform(drop_null_type))
        .into_generator()
        .into_root_schema_for::<T>();
    schema.remove("title");
    schema.remove("description");
    schema.into()
}

/// Turn `"type": ["U", "null"]` into `"type": "U"`.
fn drop_null_type(schema: &mut Schema) {
    let Some(Value::Array(types)) = schema.get_mut("type") else {
        return;
    };
    types.retain(|kind| kind != "null");
    if let [kind] = types.as_mut_slice() {
        let kind = kind.take();
        schema.insert("type".to_string(), kind);
    }
}

/// Parse the arguments sent by the model into `T`.
///
/// `null` counts as absent for an `Option` field.
///
/// # Errors
///
/// Returns an error output for the model, naming the offending field, when
/// the arguments do not match `T`: a missing required field, a value of the
/// wrong type, or a field `T` does not know when it denies unknown fields.
pub(crate) fn parse_input<T: DeserializeOwned>(input: Value) -> Result<T, ToolOutput> {
    serde_path_to_error::deserialize(input)
        .map_err(|e| ToolOutput::error(format!("Invalid arguments: {e}")))
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::json;

    use super::*;

    /// Arguments of a test tool.
    #[derive(Debug, Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct Args {
        /// What to look for
        pattern: String,
        /// Where to look
        path: Option<String>,
        /// Ignore case
        case_insensitive: Option<bool>,
    }

    #[test]
    fn schema_describes_fields_and_hides_null() {
        assert_eq!(
            input_schema::<Args>(),
            json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "What to look for" },
                    "path": { "type": "string", "description": "Where to look" },
                    "case_insensitive": { "type": "boolean", "description": "Ignore case" }
                },
                "required": ["pattern"],
                "additionalProperties": false
            })
        );
    }

    #[test]
    fn drop_null_type_keeps_a_true_union() {
        let mut schema = Schema::try_from(json!({ "type": ["string", "integer", "null"] }))
            .expect("object schema");
        drop_null_type(&mut schema);
        assert_eq!(
            Value::from(schema),
            json!({ "type": ["string", "integer"] })
        );
    }

    #[test]
    fn parse_reads_every_field() {
        let args: Args = parse_input(json!({
            "pattern": "needle",
            "path": "/srv",
            "case_insensitive": true
        }))
        .expect("valid");
        assert_eq!(args.pattern, "needle");
        assert_eq!(args.path.as_deref(), Some("/srv"));
        assert_eq!(args.case_insensitive, Some(true));
    }

    #[test]
    fn parse_treats_null_and_absent_alike() {
        let args: Args = parse_input(json!({ "pattern": "é", "path": null })).expect("valid");
        assert_eq!(args.pattern, "é");
        assert_eq!(args.path, None);
        assert_eq!(args.case_insensitive, None);
    }

    #[test]
    fn parse_names_a_mistyped_field() {
        let err = parse_input::<Args>(json!({ "pattern": "x", "case_insensitive": "yes" }))
            .expect_err("mistyped");
        assert!(err.is_error);
        assert_eq!(
            err.content,
            "Invalid arguments: case_insensitive: invalid type: string \"yes\", expected a boolean"
        );
    }

    #[test]
    fn parse_names_a_missing_field() {
        let err = parse_input::<Args>(json!({})).expect_err("missing");
        assert_eq!(err.content, "Invalid arguments: missing field `pattern`");
    }

    #[test]
    fn parse_names_an_unknown_field() {
        let err = parse_input::<Args>(json!({ "pattern": "x", "ignore_case": true }))
            .expect_err("unknown");
        assert_eq!(
            err.content,
            "Invalid arguments: ignore_case: unknown field `ignore_case`, \
             expected one of `pattern`, `path`, `case_insensitive`"
        );
    }

    #[test]
    fn parse_refuses_arguments_that_are_not_an_object() {
        let err = parse_input::<Args>(json!("needle")).expect_err("not an object");
        assert!(
            err.content.starts_with("Invalid arguments: "),
            "{}",
            err.content
        );
    }
}
