//! Compile-checks every Rust snippet in the Claude Code plugin under
//! `plugins/ironflow/`.
//!
//! Each skill document is attached as module documentation, so `cargo test
//! --doc` turns every ```` ```rust ```` block into a doctest. A change to the
//! Ironflow API that breaks a skill breaks this crate.
//!
//! The project template in `skills/setup/assets/` is a real Cargo workspace;
//! `scripts/check-plugin-template.sh` scaffolds and builds it separately.
//!
//! Run with:
//!
//! ```sh
//! cargo test -p ironflow-plugin-tests --doc
//! ```
#![allow(rustdoc::broken_intra_doc_links)]

/// `skills/ironflow/SKILL.md`
#[doc = include_str!("../../../plugins/ironflow/skills/ironflow/SKILL.md")]
pub mod hub {}

/// `skills/setup/SKILL.md`
#[doc = include_str!("../../../plugins/ironflow/skills/setup/SKILL.md")]
pub mod setup {}

/// `skills/setup/references/options.md`
#[doc = include_str!("../../../plugins/ironflow/skills/setup/references/options.md")]
pub mod setup_options {}

/// `skills/workflow/SKILL.md`
#[doc = include_str!("../../../plugins/ironflow/skills/workflow/SKILL.md")]
pub mod workflow {}

/// `skills/workflow/references/steps.md`
#[doc = include_str!("../../../plugins/ironflow/skills/workflow/references/steps.md")]
pub mod workflow_steps {}

/// `skills/workflow/references/approval-replay.md`
#[doc = include_str!("../../../plugins/ironflow/skills/workflow/references/approval-replay.md")]
pub mod workflow_approval_replay {}

/// `skills/operation/SKILL.md`
#[doc = include_str!("../../../plugins/ironflow/skills/operation/SKILL.md")]
pub mod operation {}

/// `skills/operation/references/http-json.md`
#[doc = include_str!("../../../plugins/ironflow/skills/operation/references/http-json.md")]
pub mod operation_http_json {}

/// `skills/operation/references/gitlab-issue.md`
#[doc = include_str!("../../../plugins/ironflow/skills/operation/references/gitlab-issue.md")]
pub mod operation_gitlab_issue {}

/// `skills/test/SKILL.md`
#[doc = include_str!("../../../plugins/ironflow/skills/test/SKILL.md")]
pub mod test {}
