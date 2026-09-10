# ironflow-ops-mimir

Grafana Mimir integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows.

Provides Mimir API operations (Prometheus-compatible queries, remote write, rules, cardinality analysis) as tracked workflow steps.

## Usage

```toml
[dependencies]
ironflow-ops-mimir = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_mimir::MimirClient;

// From a workflow step (reads mimir_url, mimir_token, mimir_basic_auth from the secret store)
let mimir = MimirClient::from_context(&ctx).await?;

// Explicit URL and auth
let mimir = MimirClient::new("http://mimir:9009", reqwest::Client::new())
    .with_bearer_token("my-token");

// Basic auth
let mimir = MimirClient::new("http://mimir:9009", reqwest::Client::new())
    .with_basic_auth("user", "password");
```

### Tracked operations

```rust,ignore
use ironflow_ops_mimir::MimirClient;
use ironflow_ops_mimir::query::InstantQuery;

let mimir = MimirClient::from_context(&ctx).await?;

let query = InstantQuery::new(&mimir, "up{job=\"api\"}");
let output = ctx.operation("check-api-health", &query).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `query` | Instant query, range query, metadata, exemplars |
| `series` | Series, label names, label values |
| `ingest` | Remote write (Prometheus format) |
| `rules` | Get, create, delete alerting/recording rules |
| `alerts` | Active alerts |
| `cardinality` | Label names, label values, active series |
| `compactor` | Compactor ring, tenants |
| `distributor` | Ring, all user stats |
| `ingester` | Ring, flush, shutdown |
| `status` | Build info, config, ready, memberlist |
| `store_gateway` | Ring, tenants |

## Authentication

Register `mimir_url` (required) in your workflow's secret store. Authentication is optional -- use `mimir_token` for bearer auth or `mimir_basic_auth` for basic auth (`user:password`):

```yaml
secrets:
  - name: mimir_url
    env: MIMIR_URL
  - name: mimir_token        # optional: bearer token
    env: MIMIR_TOKEN
  - name: mimir_basic_auth   # optional: "user:password"
    env: MIMIR_BASIC_AUTH
```
