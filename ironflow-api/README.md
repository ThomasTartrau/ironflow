# ironflow-api

REST API for **ironflow** run management and observability. Built with Axum.

## Architecture

| Module | Description |
|--------|-------------|
| `entities/` | DTOs and query parameter types (public API contract) |
| `routes/` | One file per route handler |
| `error.rs` | Typed API errors mapped to HTTP status codes |
| `response.rs` | Standard response envelope `{ data, meta }` |
| `state.rs` | Shared application state |
| `middleware.rs` | Authentication and worker token middleware |
| `rate_limit.rs` | Per-IP rate limiting with `governor` |
| `sse.rs` | Server-Sent Events for real-time run updates |

## API endpoints

- `POST /api/v1/runs` - Create a workflow run
- `GET /api/v1/runs` - List runs (paginated)
- `GET /api/v1/runs/:id` - Get run details
- `POST /api/v1/runs/:id/approve` - Approve a pending run
- `POST /api/v1/runs/:id/reject` - Reject a pending run
- `POST /api/v1/runs/:id/cancel` - Cancel a running workflow
- `POST /api/v1/runs/:id/retry` - Retry a failed run
- `GET /api/v1/stats` - Aggregate statistics
- `GET /api/v1/events` - SSE stream for run updates

## Feature flags

| Feature | Description |
|---------|-------------|
| `sign-up` | Enable user registration endpoint |
| `dashboard` | Embed the web dashboard via `rust-embed` |
| `prometheus` | Expose API and engine metrics |
| `openapi` | Generate OpenAPI spec via `utoipa` |

## License

MIT License - see [LICENSE](../LICENSE) for details.
