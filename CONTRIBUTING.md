# Contributing to Ironflow

## Build and test

```bash
cargo build                                  # Build the workspace
cargo test                                   # Unit and integration tests
cargo test -p ironflow-readme-tests --doc    # Compile the Rust snippets of the README and the mdBook
cargo test -p ironflow-plugin-tests --doc    # Compile the Rust snippets of the Claude Code plugin
cargo doc --no-deps                          # Docs, must be warning-free
scripts/test-postgres.sh                     # PostgreSQL suites (DATABASE_URL, or a throwaway container)
mdbook build                                 # The documentation site, from docs/book/src
```

CI also runs `cargo fmt --all -- --check` and `cargo clippy --workspace --all-targets`.

The dashboard lives in `ironflow-dashboard/`; see [its README](ironflow-dashboard/README.md)
for the frontend workflow.

## Documentation

- `README.md` explains what Ironflow is for and how to try it. Keep it short: details go in
  the mdBook.
- `docs/book/src/` is the documentation site, published at
  <https://ironflow-023e1b.gitlab.io/>.
- Every public Rust item has rustdoc, published on [docs.rs](https://docs.rs/ironflow-engine).

## Project layout

The role of each crate is listed in the
[architecture overview](docs/book/src/architecture/overview.md).
