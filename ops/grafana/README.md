# ironflow-ops-grafana

Grafana integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, using direct HTTP calls to the Grafana REST API.

Provides typed Grafana API operations (dashboards, data sources, alerting, teams, users, RBAC) as tracked workflow steps.

## Usage

```toml
[dependencies]
ironflow-ops-grafana = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_grafana::GrafanaClient;

// From a workflow step (reads grafana_token and grafana_url from the secret store)
let grafana = GrafanaClient::from_context(&ctx).await?;

// With an explicit URL (reads grafana_token from the secret store)
let grafana = GrafanaClient::from_context_with_url(&ctx, "https://grafana.example.com").await?;

// Explicit token and URL
let grafana = GrafanaClient::new("glsa_xxxx", "https://grafana.example.com")?;
```

### Tracked operations

```rust,ignore
use ironflow_ops_grafana::GrafanaClient;
use ironflow_ops_grafana::dashboards::GetDashboard;

let grafana = GrafanaClient::from_context(&ctx).await?;

let get = GetDashboard::new(&grafana, "my-dashboard-uid");
let output = ctx.operation("fetch-dashboard", &get).await?;
```

## Available operations

| Module | Operations |
|--------|------------|
| `admin` | Settings, stats |
| `alerting` | Rules, contact points, notification policies, silences, mute timings |
| `annotations` | Create, update, delete, list |
| `dashboards` | Get, create, update, delete, search |
| `data_sources` | Get, create, update, delete, list, query |
| `folders` | Get, create, update, delete, list |
| `organizations` | Get, create, update, list, members |
| `playlists` | Get, create, update, delete, list |
| `rbac` | Roles, permissions |
| `service_accounts` | Create, delete, list, tokens |
| `snapshots` | Create, delete, get |
| `teams` | Create, delete, list, members |
| `users` | Get, create, update, list, org membership |

## Authentication

Register `grafana_token` in your workflow's secret store. Optionally set `grafana_url` (defaults to `http://localhost:3000`):

```yaml
secrets:
  - name: grafana_token
    env: GRAFANA_TOKEN
  - name: grafana_url
    env: GRAFANA_URL
```
