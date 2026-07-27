//! Compile-checks every Rust snippet in the workspace `README.md`.
//!
//! This crate exists only so `cargo test --doc` compiles the README. It is the
//! single place in the workspace that may depend on `ironflow-core`,
//! `ironflow-runtime`, `ironflow-engine`, `ironflow-store` and `ironflow-sdk`
//! at once, with every optional feature enabled, which is what the README
//! snippets collectively require.
//!
//! Run it with:
//!
//! ```sh
//! cargo test -p ironflow-readme-tests --doc
//! ```
// The README links to repository files (LICENSE, the dashboard README), which rustdoc
// cannot resolve as intra-doc links. This crate's rendered documentation is not a
// deliverable, only its doctests are.
#![allow(rustdoc::broken_intra_doc_links)]
#![doc = include_str!("../../../README.md")]
