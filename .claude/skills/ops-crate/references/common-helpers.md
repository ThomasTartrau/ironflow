# ops/common -- Shared building blocks

Every HTTP-based ops crate (Pattern B) wraps `HttpApiClient` from
`ironflow-ops-common`. Pattern A and C crates do not use it.

## HttpApiClient (ops/common/src/client.rs)

```rust
use ironflow_ops_common::{Auth, HttpApiClient};
use reqwest::Client;

// Construction
let client = HttpApiClient::new("http://api.example.com/", Client::new());
// base_url trailing slash is stripped automatically

// Auth modes
let client = client.with_bearer_token("my-token");
let client = client.with_basic_auth("user", "password");

// Build from OperationContext secret store
let client = HttpApiClient::from_context(
    &ctx,
    "xxx_url",        // required -- base URL secret key
    "xxx_token",      // optional -- bearer token secret key
    "xxx_basic_auth", // optional -- "user:password" secret key
).await?;

// Request builders (all apply auth automatically)
let req = client.get("/api/v1/resource");
let req = client.post("/api/v1/resource");
let req = client.put("/api/v1/resource");
let req = client.patch("/api/v1/resource");
let req = client.delete("/api/v1/resource");

// URL builder
let url = client.url("/api/v1/status");
// "http://api.example.com/api/v1/status"
```

Key: `Auth::Debug` redacts tokens automatically -- never implement
`fmt::Debug` on a client that might print secrets.

## helpers (ops/common/src/helpers.rs)

```rust
use ironflow_ops_common::helpers::{
    check_response,       // bytes on 2xx, OperationError::Http on 4xx/5xx
    check_response_json,  // deserialize T on 2xx, handles 204 No Content
    send_request,         // maps reqwest transport errors to OperationError::Http
    parse_json_body,      // &[u8] -> Value
    to_value,             // T: Serialize -> Value
    validate_path_segment, // rejects empty, "..", "/", "\", "?", "#", spaces
    reqwest_err,          // reqwest::Error -> OperationError::Http
};
```

**NEVER duplicate these in a new crate.** Import them from
`ironflow_ops_common::helpers`.

## MapSecretResolver (test utility)

Available behind the `test-utils` feature:

```rust
// Cargo.toml
[dev-dependencies]
ironflow-ops-common = { path = "../common", features = ["test-utils"] }

// In test
use ironflow_ops_common::MapSecretResolver;
use ironflow_core::operation::OperationContext;
use std::collections::HashMap;
use std::sync::Arc;

let mut secrets = HashMap::new();
secrets.insert("xxx_url".into(), "http://localhost:8080".into());
secrets.insert("xxx_token".into(), "test-token".into());
let resolver = MapSecretResolver::new(secrets);
let ctx = OperationContext::new(Arc::new(resolver));
```

## Cargo.toml template (Pattern B)

```toml
[package]
name = "ironflow-ops-xxx"
version = "0.1.0"
edition = "2024"
rust-version = "1.94"
authors = ["Thomas Tartrau"]
license = "MIT"
description = "Xxx operations for Ironflow workflows"
repository = "https://gitlab.com/ThomasTartrau/ironflow"
keywords = ["workflow", "xxx", "operations", "ironflow"]
categories = ["asynchronous", "development-tools"]

[dependencies]
async-trait = "0.1"
ironflow-core = { path = "../../ironflow-core" }
ironflow-ops-common = { path = "../common" }
reqwest = { version = "0.13", features = ["json", "query"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[dev-dependencies]
tokio = { version = "1", features = ["full"] }
wiremock = "0.6"
```

**IMPORTANT**: use `cargo add` for versions. Never hardcode version numbers
by hand. The template above shows the structure, not exact versions.

## Workspace registration

Add the new crate to the root `Cargo.toml` workspace members:

```toml
members = [
    # ... existing members ...
    "ops/xxx",
]
```
