# Feature Flags

Optional parts of Ironflow are behind Cargo features, so a binary only compiles what it uses.

| Crate | Flag | Effect |
|-------|------|--------|
| `ironflow-core` | `prometheus` | Emit operation metrics |
| | `opentelemetry` | Export traces over OTLP |
| | `transport-ssh` | `SshProvider` (russh) |
| | `transport-docker` | `DockerProvider` (bollard) |
| | `transport-k8s` | `K8sEphemeralProvider`, `K8sPersistentProvider` (kube) |
| | `provider-anthropic-api` | Anthropic Messages API provider |
| | `provider-openai` | OpenAI provider |
| | `provider-gemini` | Google Gemini provider |
| | `provider-mistral` | Mistral provider |
| | `provider-nvidia` | NVIDIA NIM provider |
| | `provider-typesafe` | `TypeSafeProvider` for [decision steps](../concepts/decision.md) |
| | `tool-bash` | Bash tool for HTTP providers |
| | `tool-read-file` | File reading tool for HTTP providers |
| | `tool-grep` | Confined content search (regex) tool for HTTP providers |
| | `tool-glob` | Confined file-name search tool for HTTP providers |
| | `tool-web-fetch` | Web fetch tool for HTTP providers |
| | `tool-web-search` | Web search tool for HTTP providers |
| | `tool-mcp` | MCP bridge, exposes MCP servers as agent tools |
| `ironflow-store` | `store-memory` *(default)* | In-memory store, no persistence |
| | `store-postgres` | Postgres backend (sqlx) |
| | `secret-store` | AES-GCM encrypted secrets |
| | `openapi` | utoipa schemas for stored entities |
| `ironflow-api` | `dashboard` | Embed the built dashboard via `rust-embed` |
| | `sign-up` | Expose the self-service sign-up route |
| | `prometheus` | Expose `/metrics` |
| | `openapi` | Expose `/api/v1/openapi.json` |
| | `opentelemetry` | Export traces over OTLP |
| | `storage-s3` | S3 backend for [artifacts](../concepts/artifacts.md) |
| | `store-postgres` | Postgres store |
| `ironflow-engine` | `prometheus` | Engine metrics |
| | `openapi` | utoipa schemas for engine types |
| | `opentelemetry` | Export traces over OTLP |
| | `secret-store` | Resolve encrypted secrets in steps |
| `ironflow-worker` | `prometheus` | Worker metrics |
| | `opentelemetry` | Export traces over OTLP |
| | `heartbeat` | Periodic liveness reporting to the API |
| `ironflow-runtime` | `prometheus` | Webhook metrics |
| | `trigger-nats` | NATS trigger source |
| | `trigger-polling-http` | Trigger source polling an HTTP endpoint |
| | `trigger-polling-sql` | Trigger source polling a Postgres query |
| `ironflow-types` | `openapi` | utoipa schemas for envelope types |
| `ironflow-sdk` | `rustls` *(default)* | reqwest with rustls |
| | `native-tls` | reqwest with the platform TLS stack |
