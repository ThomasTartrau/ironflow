# Installation

## Prerequisites

- Rust 1.94+ (see `rust-version` in Cargo.toml)
- A running PostgreSQL instance (for production; in-memory store available for development)

## Add dependencies

Add the crates you need to your `Cargo.toml`:

```toml
[dependencies]
ironflow-engine = "0.1"   # Workflow handler, context, engine
ironflow-api = "0.1"      # REST API server
ironflow-worker = "0.1"   # Background worker
ironflow-store = "0.1"    # Storage backends
ironflow-core = "0.1"     # Shell, agent providers
```

## Minimal project structure

A typical Ironflow project has three parts:

1. **A library crate** with your workflow handlers
2. **A server binary** that exposes the API and serves the dashboard
3. **A worker binary** that executes workflows

```text
my-project/
├── src/
│   └── lib.rs          # Your workflow handlers
├── src/bin/
│   ├── server.rs       # API server
│   └── worker.rs       # Background worker
└── Cargo.toml
```

See the [example server](server.md) and [example worker](worker.md) for complete working code.
