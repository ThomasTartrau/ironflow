# ironflow-ops-postgres

PostgreSQL integration for [Ironflow](https://gitlab.com/ThomasTartrau/ironflow) workflows, powered by [`sqlx`](https://crates.io/crates/sqlx).

Provides PostgreSQL operations (queries, schema inspection, maintenance, admin tasks) as tracked workflow steps with connection pooling.

## Usage

```toml
[dependencies]
ironflow-ops-postgres = "0.1"
```

### Build a client

```rust,ignore
use ironflow_ops_postgres::PostgresClient;

// From a workflow step (reads postgres_url from the secret store)
let pg = PostgresClient::from_context(&ctx).await?;

// Explicit connection URL
let pg = PostgresClient::connect("postgres://user:pass@localhost/mydb").await?;
```

### Tracked operations

```rust,ignore
use ironflow_ops_postgres::PostgresClient;
use ironflow_ops_postgres::query::Query;

let pg = PostgresClient::from_context(&ctx).await?;

let query = Query::new(&pg, "SELECT id, name FROM users WHERE active = $1")
    .bind_bool(true);

let output = ctx.operation("list-active-users", &query).await?;
```

### Direct pool access

```rust,ignore
use ironflow_ops_postgres::PostgresClient;

let pg = PostgresClient::from_context(&ctx).await?;
let pool = pg.pool();
// Use sqlx directly with the pool
```

## Available operations

| Module | Operations |
|--------|------------|
| `query` | Query (SELECT), query one, query scalar |
| `execute` | Execute (INSERT, UPDATE, DELETE), execute many |
| `schema` | List tables, columns, indexes, constraints, extensions |
| `maintenance` | Vacuum, analyze, reindex |
| `admin` | Connections (list, terminate), database size, table stats |

## Authentication

Register `postgres_url` in your workflow's secret store:

```yaml
secrets:
  - name: postgres_url
    env: DATABASE_URL   # e.g. postgres://user:pass@host:5432/db
```
