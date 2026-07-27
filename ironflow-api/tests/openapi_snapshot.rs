//! Snapshot test: ensures `openapi.json` stays in sync with the Rust API.
//!
//! Run `UPDATE_OPENAPI=1 cargo test -p ironflow-api --all-features` to regenerate.
//!
//! Writes two files:
//! - `ironflow-dashboard/openapi.json` -- OpenAPI 3.1 (native utoipa output)
//! - `ironflow-sdk/openapi.json` -- OpenAPI 3.0.3 (for progenitor compatibility)

#![cfg(feature = "openapi")]

use ironflow_api::openapi::ApiDoc;
use std::fs;
use std::path::Path;
use utoipa::OpenApi;

use serde_json::Value;

/// Given a JSON array, find the single non-null element and whether null was present.
fn find_single_non_null(arr: &[Value], is_null: impl Fn(&Value) -> bool) -> Option<(&Value, bool)> {
    let non_null: Vec<&Value> = arr.iter().filter(|v| !is_null(v)).collect();
    if non_null.len() == 1 {
        let has_null = arr.len() > non_null.len();
        Some((non_null[0], has_null))
    } else {
        None
    }
}

/// Drop `nullable` from operation parameter schemas.
///
/// In OpenAPI 3.0 a parameter's optionality is carried by `required`, not by a
/// nullable schema. Leaving `nullable: true` there makes progenitor emit an
/// `Option<String>` it then calls `to_string()` on, which does not compile.
fn strip_parameter_nullability(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(Value::Array(params)) = map.get_mut("parameters") {
                for param in params.iter_mut() {
                    if let Some(schema) = param.get_mut("schema").and_then(|s| s.as_object_mut()) {
                        schema.remove("nullable");
                    }
                }
            }
            for val in map.values_mut() {
                strip_parameter_nullability(val);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                strip_parameter_nullability(item);
            }
        }
        _ => {}
    }
}

/// Convert OpenAPI 3.1 patterns to OpenAPI 3.0.3 for progenitor compatibility.
///
/// Handles:
/// - `"type": ["string", "null"]` -> `"type": "string", "nullable": true`
/// - `"oneOf": [{"type": "null"}, {"$ref": "..."}]` -> `{"$ref": "...", "nullable": true}`
///
/// The `"openapi"` version field is rewritten at the root level by the caller.
#[allow(clippy::collapsible_if)]
fn downgrade_31_to_30(value: &mut Value) {
    match value {
        Value::Object(map) => {
            if let Some(type_val) = map.get("type") {
                if let Some(arr) = type_val.as_array() {
                    if let Some((val, has_null)) =
                        find_single_non_null(arr, |v| v.as_str() == Some("null"))
                    {
                        map.insert("type".to_string(), val.clone());
                        if has_null {
                            map.insert("nullable".to_string(), Value::Bool(true));
                        }
                    }
                }
            }

            let is_null_schema = |v: &Value| {
                v.as_object()
                    .and_then(|o| o.get("type"))
                    .and_then(|t| t.as_str())
                    == Some("null")
            };
            let nullable_ref = map
                .get("oneOf")
                .and_then(|v| v.as_array())
                .and_then(|arr| find_single_non_null(arr, is_null_schema))
                .and_then(|(val, has_null)| if has_null { Some(val.clone()) } else { None });
            if let Some(val) = nullable_ref {
                map.remove("oneOf");
                if let Some(obj) = val.as_object() {
                    for (k, v) in obj {
                        map.insert(k.clone(), v.clone());
                    }
                }
                map.insert("nullable".to_string(), Value::Bool(true));
            }

            for val in map.values_mut() {
                downgrade_31_to_30(val);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                downgrade_31_to_30(item);
            }
        }
        _ => {}
    }
}

#[test]
fn openapi_spec_is_up_to_date() {
    let spec = ApiDoc::openapi()
        .to_pretty_json()
        .expect("failed to serialize OpenAPI spec");

    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("no parent dir");

    let dashboard_path = root.join("ironflow-dashboard/openapi.json");
    let sdk_path = root.join("ironflow-sdk/openapi.json");

    let mut spec_30: Value = serde_json::from_str(&spec).expect("failed to parse spec as JSON");
    downgrade_31_to_30(&mut spec_30);
    strip_parameter_nullability(&mut spec_30);
    if let Some(obj) = spec_30.as_object_mut() {
        obj.insert("openapi".to_string(), Value::String("3.0.3".to_string()));
    }
    let spec_30_pretty =
        serde_json::to_string_pretty(&spec_30).expect("failed to serialize 3.0 spec");

    if std::env::var("UPDATE_OPENAPI").is_ok() {
        fs::write(&dashboard_path, &spec).expect("failed to write dashboard openapi.json");
        fs::write(&sdk_path, &spec_30_pretty).expect("failed to write SDK openapi.json");
        return;
    }

    let existing_31 = fs::read_to_string(&dashboard_path).unwrap_or_else(|_| {
        panic!(
            "openapi.json not found at {}. Run: UPDATE_OPENAPI=1 cargo test -p ironflow-api --all-features",
            dashboard_path.display()
        )
    });

    assert_eq!(
        existing_31, spec,
        "openapi.json (3.1) is outdated. Run: UPDATE_OPENAPI=1 cargo test -p ironflow-api --all-features"
    );

    let existing_30 = fs::read_to_string(&sdk_path).unwrap_or_else(|_| {
        panic!(
            "openapi.json not found at {}. Run: UPDATE_OPENAPI=1 cargo test -p ironflow-api --all-features",
            sdk_path.display()
        )
    });

    assert_eq!(
        existing_30, spec_30_pretty,
        "openapi.json (3.0) is outdated. Run: UPDATE_OPENAPI=1 cargo test -p ironflow-api --all-features"
    );
}
