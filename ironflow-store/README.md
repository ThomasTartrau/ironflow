# ironflow-store

Storage abstraction and implementations for **ironflow** run tracking.

## Implementations

| Store | Feature | Description |
|-------|---------|-------------|
| `InMemoryStore` | `store-memory` (default) | In-process store, no external dependencies |
| `PostgresStore` | `store-postgres` | Production-ready with `SELECT FOR UPDATE SKIP LOCKED` |

## Additional stores

| Store | Feature | Description |
|-------|---------|-------------|
| `SecretStore` | `secret-store` | AES-256-GCM encrypted secret storage |
| `ApiKeyStore` | - | API key management with scoped permissions |
| `UserStore` | - | User account persistence |
| `AuditLogStore` | - | Audit trail for run lifecycle events |

## Feature flags

| Feature | Description |
|---------|-------------|
| `store-memory` | In-memory store (default) |
| `store-postgres` | PostgreSQL store via `sqlx` |
| `secret-store` | Encrypted secret storage (AES-GCM) |
| `openapi` | Derive `utoipa::ToSchema` on entities |

## Quick start

```rust,no_run
use ironflow_store::prelude::*;
use ironflow_store::memory::InMemoryStore;

async fn example() {
    let store = InMemoryStore::new();
    // Use store via RunStore trait methods
}
```

## License

MIT License - see [LICENSE](../LICENSE) for details.
