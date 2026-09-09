# Pattern B -- Direct HTTP calls via HttpApiClient

Use when wrapping a REST API that has no typed Rust crate. Each operation
is a standalone struct with `new()`, `run()` (typed), `execute()` (Value),
`input()`.

Reference crates: `ops/grafana`, `ops/loki`, `ops/tempo`, `ops/mimir`.

## When to use Pattern B

- No well-maintained Rust crate exists for the API
- You are calling a REST/HTTP API directly
- You need `HttpApiClient` from ops/common
- Each operation maps to one HTTP endpoint

## Structure

```
ops/xxx/
  Cargo.toml
  src/
    lib.rs            # crate doc, pub modules, pub use client::XxxClient
    client.rs         # XxxClient wrapping HttpApiClient
    module1.rs        # operations grouped by API domain
    module2.rs        # ...
  tests/
    module1.rs        # wiremock integration tests
    module2.rs        # ...
```

## Client (ops/grafana/src/client.rs pattern)

```rust
use std::fmt;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use ironflow_ops_common::HttpApiClient;
use ironflow_ops_common::helpers::{check_response_json, reqwest_err, to_value};
use reqwest::Client;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

#[derive(Clone)]
pub struct XxxClient {
    inner: HttpApiClient,
}

impl XxxClient {
    /// Build from OperationContext (reads secrets).
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let token = ctx.secrets().get("xxx_token").await?
            .ok_or_else(|| OperationError::Secret {
                message: "xxx_token secret not found".to_string(),
            })?;

        // Optional: read URL from secrets with a default
        let url = match ctx.secrets().get("xxx_url").await? {
            Some(s) => s.value.clone(),
            None => "http://localhost:3000".to_string(),
        };

        Self::new(&token.value, &url)
    }

    /// Build with explicit parameters.
    pub fn new(token: &str, base_url: &str) -> Result<Self, OperationError> {
        if token.trim().is_empty() {
            return Err(OperationError::Secret {
                message: "xxx token must not be empty".to_string(),
            });
        }

        let http = Client::builder()
            .build()
            .map_err(|e| OperationError::Http {
                status: None,
                message: format!("failed to build HTTP client: {e}"),
            })?;

        let inner = HttpApiClient::new(base_url, http).with_bearer_token(token);
        Ok(Self { inner })
    }

    pub fn base_url(&self) -> &str { self.inner.base_url() }
    pub fn url(&self, path: &str) -> String { self.inner.url(path) }

    // Internal helpers used by operations
    pub(crate) fn get_request(&self, path: &str) -> reqwest::RequestBuilder {
        self.inner.get(path)
    }

    async fn send_json<T: DeserializeOwned>(
        req: reqwest::RequestBuilder,
    ) -> Result<T, OperationError> {
        let resp = req.send().await.map_err(reqwest_err)?;
        check_response_json(resp).await
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self, path: &str,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.get(path)).await
    }

    pub(crate) async fn get_json_with_query<T: DeserializeOwned>(
        &self, path: &str, query: &[(&str, String)],
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.get(path).query(query)).await
    }

    pub(crate) async fn post_json<B: Serialize, T: DeserializeOwned>(
        &self, path: &str, body: &B,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.post(path).json(body)).await
    }

    pub(crate) async fn put_json<B: Serialize, T: DeserializeOwned>(
        &self, path: &str, body: &B,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.put(path).json(body)).await
    }

    pub(crate) async fn patch_json<B: Serialize, T: DeserializeOwned>(
        &self, path: &str, body: &B,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.patch(path).json(body)).await
    }

    pub(crate) async fn delete_json<T: DeserializeOwned>(
        &self, path: &str,
    ) -> Result<T, OperationError> {
        Self::send_json(self.inner.delete(path)).await
    }

    pub(crate) fn to_value<T: Serialize>(val: &T) -> Result<Value, OperationError> {
        to_value(val)
    }
}

// Debug MUST redact secrets
impl fmt::Debug for XxxClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("XxxClient")
            .field("base_url", &self.inner.base_url())
            .field("auth", self.inner.auth())
            .finish()
    }
}
```

## Operation struct (ops/grafana/src/annotations.rs extract)

Each operation follows this exact structure:

```rust
use async_trait::async_trait;
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext, TypedOperation};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::client::XxxClient;

/// Response type.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceCreateOutput {
    pub id: Option<u64>,
    pub message: Option<String>,
}

/// Create a resource.
///
/// Sends a `POST /api/resources` request.
pub struct ResourceCreate {
    client: XxxClient,
    body: Value,
}

impl ResourceCreate {
    pub fn new(client: &XxxClient, body: Value) -> Self {
        Self { client: client.clone(), body }
    }

    /// Execute and return a typed result.
    pub async fn run(&self) -> Result<ResourceCreateOutput, OperationError> {
        self.client.post_json("/api/resources", &self.body).await
    }
}

#[async_trait]
impl Operation for ResourceCreate {
    fn kind(&self) -> &str { "xxx" }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        XxxClient::to_value(&self.run().await?)
    }

    fn input(&self) -> Option<Value> {
        Some(self.body.clone())
    }
}

impl TypedOperation for ResourceCreate {
    type Output = ResourceCreateOutput;
}
```

Checklist for every operation:
- `kind()` returns the service name (lowercase, no prefix)
- `execute()` calls `run()` then serializes to `Value`
- `input()` returns params for audit -- NEVER include secrets
- `run()` returns the typed response directly
- `TypedOperation` associate type matches `run()` return

## Wiremock integration test (ops/grafana/tests/other.rs pattern)

```rust
use std::sync::Arc;

use ironflow_core::operation::{NoopSecretResolver, Operation, OperationContext};
use ironflow_ops_xxx::XxxClient;
use ironflow_ops_xxx::resources::ResourceCreate;
use serde_json::json;
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn setup() -> (MockServer, XxxClient, OperationContext) {
    let server = MockServer::start().await;
    let client = XxxClient::new("test-token", &server.uri()).unwrap();
    let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
    (server, client, ctx)
}

#[tokio::test]
async fn create_resource() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/resources"))
        .and(bearer_token("test-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 1, "message": "created"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let op = ResourceCreate::new(&client, json!({"name": "test"}));
    assert_eq!(op.kind(), "xxx");
    let result = op.execute(&ctx).await.unwrap();
    assert_eq!(result["id"], 1);
}

#[tokio::test]
async fn create_resource_typed() {
    let (server, client, _ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/resources"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": 2, "message": "ok"
        })))
        .mount(&server)
        .await;

    let op = ResourceCreate::new(&client, json!({"name": "test"}));
    let output = op.run().await.unwrap();
    assert_eq!(output.id, Some(2));
}

#[tokio::test]
async fn create_resource_4xx() {
    let (server, client, ctx) = setup().await;
    Mock::given(method("POST"))
        .and(path("/api/resources"))
        .respond_with(ResponseTemplate::new(400).set_body_string("bad request"))
        .mount(&server)
        .await;

    let op = ResourceCreate::new(&client, json!({}));
    let err = op.execute(&ctx).await.unwrap_err();
    assert!(err.to_string().contains("bad request"));
}
```

## Client tests (always included)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ironflow_core::operation::{NoopSecretResolver, OperationContext};

    #[tokio::test]
    async fn from_context_fails_when_token_missing() {
        let ctx = OperationContext::new(Arc::new(NoopSecretResolver));
        let err = XxxClient::from_context(&ctx).await.unwrap_err();
        assert!(err.to_string().contains("xxx_token"));
    }

    #[test]
    fn normalizes_base_url_trailing_slash() {
        let client = XxxClient::new("tok", "https://xxx.example.com/").unwrap();
        assert_eq!(client.base_url(), "https://xxx.example.com");
    }

    #[test]
    fn debug_does_not_leak_token() {
        let client = XxxClient::new("super-secret", "https://xxx.example.com").unwrap();
        let debug = format!("{client:?}");
        assert!(!debug.contains("super-secret"));
        assert!(debug.contains("redacted"));
    }

    #[test]
    fn rejects_empty_token() {
        let err = XxxClient::new("", "https://xxx.example.com").unwrap_err();
        assert!(err.to_string().contains("empty"));
    }
}
```
