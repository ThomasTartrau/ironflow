# ironflow-dashboard

Web UI for the [Ironflow](../README.md) workflow orchestration platform. It talks to
`ironflow-api` over REST and consumes the SSE event stream for live run updates.

## What it covers

| Area | Route | Description |
|------|-------|-------------|
| Dashboard | `/` | Run activity and aggregate statistics |
| Workflows | `/workflows` | Catalog, per-workflow detail, source code, and a form generated from the handler's `input_schema` |
| Runs | `/runs` | History with status and workflow filters, step timeline, live logs, approve/reject/cancel/retry |
| Secrets | `/secrets` | Create and delete encrypted secrets (values are never returned by the API) |
| API keys | `/api-keys` | Issue and revoke scoped keys for the CLI, SDK and MCP server |
| Users | `/users` | User administration |
| Auth | `/sign-in`, `/sign-up` | JWT session, sign-up only when the API is built with the `sign-up` feature |

## Stack

React 19, React Router 7, Redux Toolkit, Tailwind CSS 4, Base UI, Vite, Biome, Vitest.

API types in `src/app/lib/types.generated.ts` are generated from `openapi.json`, so the UI cannot
drift from the API contract.

## Development

```bash
pnpm install
pnpm dev
```

Vite serves on <http://localhost:5173> and proxies `/api` to `http://localhost:3000`. Point it at
another API with `VITE_API_URL`:

```bash
VITE_API_URL=http://localhost:8080 pnpm dev
```

You need a running API for anything beyond the sign-in screen - see the platform quick start in
the [root README](../README.md#-quick-start).

## Scripts

| Command | Description |
|---------|-------------|
| `pnpm dev` | Dev server with HMR |
| `pnpm build` | Type-check then build to `dist/` |
| `pnpm preview` | Serve the production build locally |
| `pnpm test` | Run the test suite once (Vitest) |
| `pnpm test:watch` | Watch mode |
| `pnpm test:coverage` | Coverage report |
| `pnpm lint` | Biome check |
| `pnpm format` | Biome format, writes in place |
| `pnpm typecheck` | `tsc -b --noEmit` |
| `pnpm generate:types` | Regenerate `types.generated.ts` from `openapi.json` |

CI runs `pnpm lint` and `pnpm tsc -b --noEmit` on every merge request.

## How it gets served in production

`ironflow-api` embeds the built assets with `rust-embed` when compiled with its `dashboard`
feature. The build script resolves the asset directory in this order:

1. `IRONFLOW_DASHBOARD_DIR`, if set at compile time.
2. `ironflow-api/dashboard/`, which is where CI copies the build before publishing to crates.io.
3. `ironflow-dashboard/dist/`, the monorepo layout.

So in this repository, run `pnpm build` before building the API with the `dashboard` feature -
otherwise `rust-embed` has nothing to embed.

At runtime, setting `DASHBOARD_DIR` on the server makes it serve from that directory instead of
the embedded copy, which is useful for swapping the UI without rebuilding the binary.

Unknown paths fall back to `index.html` so client-side routing works on a hard refresh.

## Regenerating API types

After any change to the API surface, refresh `openapi.json` from a running server and regenerate:

```bash
curl http://localhost:3000/api/v1/openapi.json -o openapi.json
pnpm generate:types
```
