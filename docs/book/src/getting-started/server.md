# Running the Server

The API server exposes a REST API for managing workflows, runs, and steps. It also serves the web dashboard.

## Example server

The repository includes a complete example server:

```rust,ignore
{{#include ../../../../examples/server/src/main.rs}}
```

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `IRONFLOW_ENV` | `development` | `production` or `development` |
| `DATABASE_URL` | -- | PostgreSQL URL (required in production) |
| `JWT_SECRET` | dev secret | JWT signing key (**required in production, do not use the default**) |
| `WORKER_TOKEN` | dev token | Shared secret for worker auth (**required in production, do not use the default**) |
| `PORT` | `3000` | HTTP listen port |
| `ALLOWED_ORIGINS` | same-origin | Comma-separated CORS origins |
| `ARTIFACTS_DIR` | -- | Filesystem root for step artifacts |

## Running

```sh
cargo run -p ironflow-example-server
```

The server starts on `http://localhost:3000`. The dashboard is available at the root URL.
