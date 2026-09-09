# Pattern C -- Macro-generated operations from a typed Rust library

Use when a Rust crate exposes typed methods 1:1 with the API (e.g.
`slack_morphism`). A macro generates the boilerplate for each operation
struct.

Reference crate: `ops/slack`.

## When to use Pattern C

- A typed Rust crate exists and exposes one method per API call
- Methods take a typed request struct, return a typed response struct
- You want to wrap dozens of methods with minimal boilerplate
- The crate handles HTTP internally (you do NOT need HttpApiClient)

## Structure

```
ops/xxx/
  Cargo.toml
  src/
    lib.rs        # crate doc, re-exports
    client.rs     # XxxClient with from_context, session()
    error.rs      # error conversion from the underlying crate
    macros.rs     # xxx_op! macro + to_value helper
    module1.rs    # operations grouped by API domain (uses the macro)
    module2.rs
```

## The macro (ops/slack/src/macros.rs)

```rust
// Helper: serialize to Value
pub(crate) fn to_value<T: serde::Serialize>(
    val: &T,
) -> Result<serde_json::Value, ironflow_core::error::OperationError> {
    serde_json::to_value(val).map_err(|e| ironflow_core::error::OperationError::External {
        origin: "xxx".to_string(),
        message: format!("failed to serialize response: {e}"),
    })
}

/// Generate an operation struct wrapping one API method.
///
/// With request parameter:
/// ```ignore
/// xxx_op! {
///     /// Doc comment for the operation.
///     OpName => method_name(RequestType) -> ResponseType
/// }
/// ```
///
/// Without request parameter:
/// ```ignore
/// xxx_op! {
///     /// Doc comment.
///     OpName => method_name() -> ResponseType
/// }
/// ```
macro_rules! xxx_op {
    // With request parameter
    (
        $(#[$meta:meta])*
        $name:ident => $method:ident( $req:ty ) -> $resp:ty
    ) => {
        $(#[$meta])*
        pub struct $name {
            client: $crate::XxxClient,
            request: $req,
        }

        impl $name {
            pub fn new(client: &$crate::XxxClient, request: $req) -> Self {
                Self { client: client.clone(), request }
            }

            pub async fn run(&self) -> Result<$resp, ironflow_core::error::OperationError> {
                let session = self.client.session();
                session.$method(&self.request).await
                    .map_err($crate::error::from_xxx_error)
            }
        }

        #[async_trait::async_trait]
        impl ironflow_core::operation::Operation for $name {
            fn kind(&self) -> &str { "xxx" }

            async fn execute(
                &self,
                _ctx: &ironflow_core::operation::OperationContext,
            ) -> Result<serde_json::Value, ironflow_core::error::OperationError> {
                let result = self.run().await?;
                $crate::macros::to_value(&result)
            }

            fn input(&self) -> Option<serde_json::Value> {
                serde_json::to_value(&self.request).ok()
            }
        }

        impl ironflow_core::operation::TypedOperation for $name {
            type Output = $resp;
        }
    };

    // Without request parameter (parameterless API call)
    (
        $(#[$meta:meta])*
        $name:ident => $method:ident() -> $resp:ty
    ) => {
        $(#[$meta])*
        pub struct $name {
            client: $crate::XxxClient,
        }

        impl $name {
            pub fn new(client: &$crate::XxxClient) -> Self {
                Self { client: client.clone() }
            }

            pub async fn run(&self) -> Result<$resp, ironflow_core::error::OperationError> {
                let session = self.client.session();
                session.$method().await
                    .map_err($crate::error::from_xxx_error)
            }
        }

        #[async_trait::async_trait]
        impl ironflow_core::operation::Operation for $name {
            fn kind(&self) -> &str { "xxx" }

            async fn execute(
                &self,
                _ctx: &ironflow_core::operation::OperationContext,
            ) -> Result<serde_json::Value, ironflow_core::error::OperationError> {
                let result = self.run().await?;
                $crate::macros::to_value(&result)
            }

            fn input(&self) -> Option<serde_json::Value> { None }
        }

        impl ironflow_core::operation::TypedOperation for $name {
            type Output = $resp;
        }
    };
}

pub(crate) use xxx_op;
```

## Error conversion module (ops/slack/src/error.rs pattern)

```rust
use ironflow_core::error::OperationError;

pub fn from_xxx_error(e: xxx_crate::Error) -> OperationError {
    OperationError::External {
        origin: "xxx".to_string(),
        message: e.to_string(),
    }
}
```

## Usage in a module (ops/slack/src/chat.rs pattern)

```rust
use xxx_crate::api::{
    DoThingRequest, DoThingResponse,
    ListThingsRequest, ListThingsResponse,
};

use crate::macros::xxx_op;

xxx_op! {
    /// Do a thing.
    ///
    /// Wraps [`things.do`](https://api.example.com/methods/things.do).
    DoThing => do_thing(DoThingRequest) -> DoThingResponse
}

xxx_op! {
    /// List things.
    ListThings => list_things(ListThingsRequest) -> ListThingsResponse
}
```

## Client (ops/slack/src/client.rs pattern)

```rust
use std::fmt;
use std::sync::Arc;

use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;

#[derive(Clone)]
pub struct XxxClient {
    inner: Arc<xxx_crate::Client>,
    token: xxx_crate::Token,
}

impl XxxClient {
    pub async fn from_context(ctx: &OperationContext) -> Result<Self, OperationError> {
        let secret = ctx.secrets().get("xxx_token").await?
            .ok_or_else(|| OperationError::Secret {
                message: "xxx_token secret not found".to_string(),
            })?;
        Self::new(&secret.value)
    }

    pub fn new(token: &str) -> Result<Self, OperationError> {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(OperationError::Secret {
                message: "xxx token must not be empty".to_string(),
            });
        }
        // Build the underlying client
        let client = xxx_crate::Client::new(/* connector */);
        let token = xxx_crate::Token::new(trimmed.to_string());
        Ok(Self { inner: Arc::new(client), token })
    }

    /// Open a session for making API calls.
    pub fn session(&self) -> xxx_crate::Session<'_> {
        self.inner.session(&self.token)
    }
}

impl fmt::Debug for XxxClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("XxxClient")
            .field("client", &"[XxxClient]")
            .finish()
    }
}
```

## Tests

Pattern C tests are mostly `#[ignore]` (need real API) except:
- `from_context_fails_when_token_missing`
- `new_rejects_empty_token`
- `new_rejects_whitespace_token`
- `debug_does_not_leak_token`
- Macro `to_value` unit tests (serialize a struct, serialize unit)
