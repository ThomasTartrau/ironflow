# ironflow-ops-loki

Grafana Loki integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows.

Provides Loki API operations (query, ingest, rules, labels, patterns, rings, status) as tracked workflow steps.

## Usage

```toml
[dependencies]
ironflow-ops-loki = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_loki::LokiClient;

// From a workflow step (reads loki_url, loki_token, loki_basic_auth from the secret store)
let loki = LokiClient::from_context(&ctx).await?;

// Explicit URL and auth
let loki = LokiClient::new("http://loki:3100", reqwest::Client::new())
    .with_bearer_token("my-token");

// Basic auth
let loki = LokiClient::new("http://loki:3100", reqwest::Client::new())
    .with_basic_auth("user", "password");
```

### Tracked operations

```rust,ignore
use ironflow_ops_loki::LokiClient;
use ironflow_ops_loki::query::QueryRange;

let loki = LokiClient::from_context(&ctx).await?;

let query = QueryRange::new(&loki, r#"{app="my-service"} |= "error""#)
    .limit(100);

let output = ctx.operation("query-errors", &query).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `query` | Query, query range, series, tail |
| `labels` | Labels, label values |
| `ingest` | Push log entries |
| `delete` | Delete log entries |
| `rules` | Get, create, delete ruler rules |
| `rings` | Ring status |
| `index` | Index stats, volume |
| `ingester` | Flush, shutdown |
| `patterns` | Query log patterns |
| `status` | Build info, config, ready, ring |
| `format` | Detect log format |

## Authentication

Register `loki_url` (required) in your workflow's secret store. Authentication is optional -- use `loki_token` for bearer auth or `loki_basic_auth` for basic auth (`user:password`):

```yaml
secrets:
  - name: loki_url
    env: LOKI_URL
  - name: loki_token        # optional: bearer token
    env: LOKI_TOKEN
  - name: loki_basic_auth   # optional: "user:password"
    env: LOKI_BASIC_AUTH
```
