# Setup options

Edits to apply on top of the scaffold, one block per answer. Paths are relative to the
project root. Each snippet is compiled in Ironflow's CI.

## Postgres instead of the in-memory store

```bash
cargo add -p server ironflow-store --features store-postgres,secret-store
```

In `server/src/main.rs`, replace the `InMemoryStore` construction:

```rust,no_run
use std::env;
use std::process;

use ironflow_store::postgres::PostgresStore;

async fn build_store() -> PostgresStore {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| {
        eprintln!("DATABASE_URL must be set");
        process::exit(1);
    });
    // Connects, verifies the connection, and runs the bundled migrations.
    PostgresStore::new(&database_url).await.unwrap_or_else(|e| {
        eprintln!("cannot connect to postgres: {e}");
        process::exit(1);
    })
}
```

`PostgresStore` has the same `set_key_ring` method as `InMemoryStore`, so the key ring
block stays unchanged. Uncomment `DATABASE_URL` in `.env`. A local database:

```bash
docker run -d --name ironflow-pg -e POSTGRES_USER=ironflow -e POSTGRES_PASSWORD=ironflow \
  -e POSTGRES_DB=ironflow -p 5432:5432 postgres:17
```

## Agent provider on the worker

The worker owns the provider. The server keeps `ClaudeCodeProvider`; it never runs an
agent step.

### Anthropic API

```bash
cargo add -p worker ironflow-core --features provider-anthropic-api
```

```rust,no_run
use std::sync::Arc;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::http::AnthropicApiProvider;

fn provider() -> Arc<dyn AgentProvider> {
    // Panics when ANTHROPIC_API_KEY is unset: fail at boot, not at the first run.
    Arc::new(AnthropicApiProvider::from_env())
}
```

Uncomment `ANTHROPIC_API_KEY` in `.env`.

### OpenAI

```bash
cargo add -p worker ironflow-core --features provider-openai
```

```rust,no_run
use std::sync::Arc;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::http::OpenAiProvider;

fn provider() -> Arc<dyn AgentProvider> {
    Arc::new(OpenAiProvider::from_env())
}
```

Uncomment `OPENAI_API_KEY` in `.env`.

### Tools for HTTP providers

HTTP providers have no CLI to run tools for them. Enable what the workflows need:

```bash
cargo add -p worker ironflow-core --features tool-bash,tool-read-file,tool-web-fetch
```

### Mixing providers

`ProviderRouter` dispatches on the model name, so one worker can serve several vendors:

```rust,no_run
use std::sync::Arc;

use ironflow_core::provider::AgentProvider;
use ironflow_core::providers::claude::ClaudeCodeProvider;
use ironflow_core::providers::http::AnthropicApiProvider;
use ironflow_core::providers::router::{ProviderMatcher, ProviderRouter};

fn provider() -> Arc<dyn AgentProvider> {
    let cli = Arc::new(ClaudeCodeProvider::new());
    let api = Arc::new(AnthropicApiProvider::from_env());
    Arc::new(
        ProviderRouter::new(cli).route(ProviderMatcher::ModelPrefix("claude-".into()), api),
    )
}
```

## Production checklist

`IRONFLOW_ENV=production` makes the server refuse to boot without `DATABASE_URL`,
`JWT_SECRET` and `WORKER_TOKEN`. Generate secrets with `openssl rand -hex 32`. Set
`IRONFLOW_SECRET_KEYS` before the first workflow that reads a secret; the key rotation
procedure is in the Ironflow README.
