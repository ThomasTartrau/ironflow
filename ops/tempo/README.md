# ironflow-ops-tempo

Grafana Tempo integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows.

Provides Tempo API operations (trace queries, search, metrics, cluster status, overrides) as tracked workflow steps.

## Usage

```toml
[dependencies]
ironflow-ops-tempo = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_tempo::TempoClient;

// From a workflow step (reads tempo_url, tempo_token, tempo_basic_auth from the secret store)
let tempo = TempoClient::from_context(&ctx).await?;

// Explicit URL and auth
let tempo = TempoClient::new("http://tempo:3200", reqwest::Client::new())
    .with_bearer_token("my-token");

// Basic auth
let tempo = TempoClient::new("http://tempo:3200", reqwest::Client::new())
    .with_basic_auth("user", "password");
```

### Tracked operations

```rust,ignore
use ironflow_ops_tempo::TempoClient;
use ironflow_ops_tempo::traces::GetTrace;

let tempo = TempoClient::from_context(&ctx).await?;

let trace = GetTrace::new(&tempo, "abc123def456");
let output = ctx.operation("fetch-trace", &trace).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `traces` | Get trace by ID, search traces, search tags, search tag values |
| `metrics` | Query range, summary |
| `cluster` | Ring status, memberlist |
| `status` | Build info, config, ready |
| `maintenance` | Flush, shutdown, compaction |
| `overrides` | Get, set, delete per-tenant overrides |
| `tags` | Tag names, tag values (v2 API) |

## Authentication

Register `tempo_url` (required) in your workflow's secret store. Authentication is optional -- use `tempo_token` for bearer auth or `tempo_basic_auth` for basic auth (`user:password`):

```yaml
secrets:
  - name: tempo_url
    env: TEMPO_URL
  - name: tempo_token        # optional: bearer token
    env: TEMPO_TOKEN
  - name: tempo_basic_auth   # optional: "user:password"
    env: TEMPO_BASIC_AUTH
```
