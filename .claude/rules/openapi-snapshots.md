# OpenAPI Snapshots -- CI Feature Parity

The CI pipeline compiles the workspace with a specific feature set:

```
IRONFLOW_FEATURES="prometheus,openapi,sign-up,transport-docker,transport-k8s,transport-ssh,secret-store"
```

Features like `sign-up` gate API routes behind `#[cfg(feature = "...")]`. The OpenAPI
spec is generated at test time by utoipa, so missing features = missing routes = snapshot
mismatch that passes locally but fails in CI.

## Regenerating snapshots

Always use the full CI feature set:

```bash
UPDATE_OPENAPI=1 cargo test --workspace --exclude ironflow-example-server \
  --features "prometheus,openapi,sign-up,transport-docker,transport-k8s,transport-ssh" \
  -- openapi_spec_is_up_to_date
```

Then regenerate the dashboard TypeScript types:

```bash
cd ironflow-dashboard && pnpm generate:types
```

## When to regenerate

Any change that touches:
- `RunStatus`, `StepStatus`, or any `#[derive(utoipa::ToSchema)]` type
- API route handlers or their `#[utoipa::path]` annotations
- Response/request structs exposed via OpenAPI

must be followed by a full regeneration with the commands above.
