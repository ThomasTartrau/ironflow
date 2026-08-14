//! # ironflow-templates
//!
//! Template system for the [ironflow](https://gitlab.com/ThomasTartrau/ironflow)
//! workflow engine, inspired by [shadcn/ui](https://ui.shadcn.com).
//!
//! Templates are workflow handlers hosted in external Git repositories.
//! Instead of installing them as dependencies, `ironflow-cli template add`
//! copies the source files directly into your project -- you own the code
//! and can customize it freely.
//!
//! # Template format
//!
//! A template repository contains one or more template directories, each
//! with a `template.toml` manifest and a `src/` folder:
//!
//! ```text
//! my-templates/
//!   ci-pipeline/
//!     template.toml
//!     src/
//!       ci_pipeline.rs
//!   code-review/
//!     template.toml
//!     src/
//!       code_review.rs
//! ```
//!
//! # Quick start
//!
//! ```no_run
//! use std::path::Path;
//! use ironflow_templates::registry::discover_templates;
//!
//! # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
//! let templates = discover_templates(Path::new("./my-templates"))?;
//! for (name, (manifest, _dir)) in &templates {
//!     println!("{name}: {}", manifest.template.description);
//! }
//! # Ok(())
//! # }
//! ```

pub mod error;
pub mod fetch;
pub mod install;
pub mod manifest;
pub mod registry;
