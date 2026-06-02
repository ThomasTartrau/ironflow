# ironflow-types

Shared API envelope types for the **ironflow** ecosystem. Used by both the server (`ironflow-api`) and the client SDK (`ironflow-sdk`) to ensure consistent request/response serialization.

## Types

| Type | Description |
|------|-------------|
| `ApiResponse<T>` | Standard response envelope: `{ data: T, meta: { page, per_page, total } }` |
| `ApiMeta` | Pagination metadata for list endpoints |
| `ErrorEnvelope` | Structured error body: `{ code, message }` |

## Feature flags

| Feature | Description |
|---------|-------------|
| `openapi` | Derive `utoipa::ToSchema` for OpenAPI spec generation |

## Usage

```rust
use ironflow_types::{ApiResponse, ApiMeta};

let response = ApiResponse {
    data: vec!["item1", "item2"],
    meta: Some(ApiMeta::paginated(1, 10, 42)),
};
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
