# ironflow-ops-common

Shared HTTP client, helpers, and test utilities for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) ops crates.

This crate is not an integration you use directly in workflows. It provides the building blocks that HTTP-based ops crates (Grafana, Loki, Mimir, Tempo) share internally: a multi-auth HTTP client, response-checking helpers, and JSON utilities.

## Usage

```toml
[dependencies]
ironflow-ops-common = "0.1"
```

### HTTP client with authentication

```rust,ignore
use ironflow_ops_common::HttpApiClient;
use reqwest::Client;

let client = HttpApiClient::new("http://api.example.com", Client::new())
    .with_bearer_token("my-token");

let req = client.get("/api/v1/resource");
```

### Authentication modes

```rust,ignore
use ironflow_ops_common::Auth;

let none = Auth::None;
let bearer = Auth::Bearer("token".into());
let basic = Auth::Basic {
    user: "admin".into(),
    password: "secret".into(),
};
```

### From a workflow context

When building a product-specific ops crate, use `from_context` to resolve credentials from the secret store:

```rust,ignore
use ironflow_ops_common::HttpApiClient;
use ironflow_core::operation::OperationContext;

let client = HttpApiClient::from_context(
    &ctx,
    "my_service_url",    // secret key for the base URL
    "my_service_token",  // secret key for the bearer token
    "my_service_basic",  // secret key for basic auth (user:password)
).await?;
```

## Helpers

The `helpers` module provides:

- `check_response_json` -- verify an HTTP response status and deserialize JSON
- `reqwest_err` -- convert a `reqwest::Error` into an `OperationError`
- `to_value` -- serialize any `Serialize` into `serde_json::Value`

## Test utilities

Enable the `test-utils` feature for `MapSecretResolver`, an in-memory secret store for testing:

```toml
[dev-dependencies]
ironflow-ops-common = { version = "0.1", features = ["test-utils"] }
```
