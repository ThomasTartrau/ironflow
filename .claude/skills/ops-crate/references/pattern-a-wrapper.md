# Pattern A -- Generic wrapper around a typed Rust library

Use when a Rust crate already exists for the service and exposes a uniform
trait (e.g. `gitlab::Endpoint`, `kube::Api`). The ops crate is thin: a
client struct, a generic `Op<E>` that implements `Operation`, and re-exports.

Reference crates: `ops/gitlab`, `ops/k8s`.

## When to use Pattern A

- A well-maintained Rust crate exists on crates.io
- The crate exposes typed endpoint builders or resource APIs
- The crate handles serialization/deserialization internally
- You do NOT need `HttpApiClient` from ops/common

## Structure

```
ops/xxx/
  Cargo.toml
  src/
    lib.rs        # crate doc, re-exports, pub use xxx_crate
    client.rs     # XxxClient with from_context + op() method
    operation.rs  # XxxOp<E> generic Operation wrapper
```

## Client (ops/gitlab/src/client.rs extract)

```rust
use gitlab::{AsyncGitlab, GitlabBuilder};
use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;

pub struct GitLab {
    inner: AsyncGitlab,
}

impl GitLab {
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        Self::from_context_with_host(ctx, "gitlab.com").await
    }

    pub async fn from_context_with_host(
        ctx: &OperationContext,
        host: &str,
    ) -> Result<Self, OperationError> {
        let secret = ctx.secrets().get("gitlab_token").await?
            .ok_or_else(|| OperationError::Secret {
                message: "gitlab_token secret not found".to_string(),
            })?;
        Self::new(&secret.value, host).await
    }

    pub async fn new(token: &str, host: &str) -> Result<Self, OperationError> {
        let inner = GitlabBuilder::new(host, token)
            .build_async()
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: e.to_string(),
            })?;
        Ok(Self { inner })
    }

    pub fn client(&self) -> &AsyncGitlab { &self.inner }

    pub fn op<E>(&self, endpoint: E) -> GitLabOp<E> {
        GitLabOp::new(self.inner.clone(), endpoint)
    }
}

// Debug MUST redact secrets
impl std::fmt::Debug for GitLab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GitLab")
            .field("client", &"[AsyncGitlab]")
            .finish()
    }
}
```

Key points:
- `from_context` reads a single secret (the token)
- `op()` wraps any endpoint as a tracked `Operation`
- `client()` exposes the inner client for direct typed queries
- Re-export the underlying crate: `pub use gitlab;`

## Generic Operation (ops/gitlab/src/operation.rs extract)

```rust
use async_trait::async_trait;
use gitlab::AsyncGitlab;
use gitlab::api::{AsyncQuery, Endpoint};
use ironflow_core::error::OperationError;
use ironflow_core::operation::{Operation, OperationContext};
use serde_json::Value;

pub struct GitLabOp<E> {
    client: AsyncGitlab,
    endpoint: E,
}

impl<E> GitLabOp<E> {
    pub(crate) fn new(client: AsyncGitlab, endpoint: E) -> Self {
        Self { client, endpoint }
    }
}

#[async_trait]
impl<E> Operation for GitLabOp<E>
where
    E: Endpoint + Sync + Send,
{
    fn kind(&self) -> &str { "gitlab" }

    async fn execute(&self, _ctx: &OperationContext) -> Result<Value, OperationError> {
        let result: Value = self.endpoint
            .query_async(&self.client)
            .await
            .map_err(|e| OperationError::Http {
                status: None,
                message: e.to_string(),
            })?;
        Ok(result)
    }

    fn input(&self) -> Option<Value> {
        Some(Value::Object(serde_json::Map::from_iter([(
            "endpoint".to_string(),
            Value::String(self.endpoint.endpoint().into_owned()),
        )])))
    }
}
```

Key points:
- Generic over `E` with a trait bound from the underlying crate
- `kind()` returns the service name (used for step tracking)
- `input()` returns metadata for audit, NEVER secrets
- `execute()` deserializes to `Value` for the engine

## lib.rs pattern

```rust
//! Xxx integration for Ironflow workflows.
//! ... crate-level doc ...

mod client;
mod operation;

pub use client::XxxClient;
pub use xxx_crate; // re-export the underlying crate
pub use operation::XxxOp;
```

## Tests

Pattern A tests are mostly `#[ignore]` (need real API) except:
- `from_context_fails_when_token_missing` (fast, no network)
- `debug_does_not_leak_token` (can mark `#[ignore]` if it needs network)
