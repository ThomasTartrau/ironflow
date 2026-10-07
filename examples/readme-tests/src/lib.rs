//! Compile-checks the Rust snippets of the workspace `README.md` and of the mdBook
//! pages that are not built from `examples/`.
//!
//! This crate exists only so `cargo test --doc` compiles that documentation. It is
//! the single place in the workspace that may depend on `ironflow-core`,
//! `ironflow-runtime`, `ironflow-engine`, `ironflow-store` and `ironflow-sdk` at
//! once, with every optional feature enabled, which is what the snippets
//! collectively require.
//!
//! Run it with:
//!
//! ```sh
//! cargo test -p ironflow-readme-tests --doc
//! ```
// The pages link to repository files and to other pages, which rustdoc cannot
// resolve as intra-doc links. This crate's rendered documentation is not a
// deliverable, only its doctests are.
#![allow(rustdoc::broken_intra_doc_links)]
#![doc = include_str!("../../../README.md")]

/// `docs/book/src/concepts/runs.md`
#[doc = include_str!("../../../docs/book/src/concepts/runs.md")]
pub mod book_runs {}

/// `docs/book/src/concepts/artifacts.md`
#[doc = include_str!("../../../docs/book/src/concepts/artifacts.md")]
pub mod book_artifacts {}

/// `docs/book/src/guides/agent-providers.md`
#[doc = include_str!("../../../docs/book/src/guides/agent-providers.md")]
pub mod book_agent_providers {}

/// `docs/book/src/guides/library-mode.md`
#[doc = include_str!("../../../docs/book/src/guides/library-mode.md")]
pub mod book_library_mode {}

/// `docs/book/src/guides/standalone-runtime.md`
#[doc = include_str!("../../../docs/book/src/guides/standalone-runtime.md")]
pub mod book_standalone_runtime {}

/// `docs/book/src/reference/interfaces.md`
#[doc = include_str!("../../../docs/book/src/reference/interfaces.md")]
pub mod book_interfaces {}
