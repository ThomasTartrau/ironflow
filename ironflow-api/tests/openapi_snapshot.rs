//! Snapshot test: ensures `openapi.json` stays in sync with the Rust API.
//!
//! Run `UPDATE_OPENAPI=1 cargo test -p ironflow-api --features openapi` to regenerate.

#![cfg(feature = "openapi")]

use ironflow_api::openapi::ApiDoc;
use std::fs;
use std::path::Path;
use utoipa::OpenApi;

#[test]
fn openapi_spec_is_up_to_date() {
    let spec = ApiDoc::openapi()
        .to_pretty_json()
        .expect("failed to serialize OpenAPI spec");

    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("no parent dir")
        .join("ironflow-dashboard/openapi.json");

    if std::env::var("UPDATE_OPENAPI").is_ok() {
        fs::write(&path, &spec).expect("failed to write openapi.json");
        return;
    }

    let existing = fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "openapi.json not found at {}. Run: UPDATE_OPENAPI=1 cargo test -p ironflow-api --features openapi",
            path.display()
        )
    });

    assert_eq!(
        existing, spec,
        "openapi.json is outdated. Run: UPDATE_OPENAPI=1 cargo test -p ironflow-api --features openapi"
    );
}
