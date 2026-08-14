//! Template manifest parsing (`template.toml`).
//!
//! Each template directory contains a `template.toml` file describing
//! the template metadata and its Rust dependencies.
//!
//! # Examples
//!
//! ```
//! use ironflow_templates::manifest::TemplateManifest;
//!
//! let toml = r#"
//! [template]
//! name = "ci-pipeline"
//! description = "Generic CI pipeline"
//! version = "1.0.0"
//! "#;
//!
//! let manifest = TemplateManifest::parse(toml)?;
//! assert_eq!(manifest.template.name, "ci-pipeline");
//! # Ok::<(), ironflow_templates::error::TemplateError>(())
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::Deserialize;

use crate::error::TemplateError;

/// Filename expected inside each template directory.
pub const MANIFEST_FILENAME: &str = "template.toml";

/// Top-level manifest parsed from `template.toml`.
///
/// # Examples
///
/// ```
/// use ironflow_templates::manifest::TemplateManifest;
///
/// let toml = r#"
/// [template]
/// name = "code-review"
/// description = "Automated code review"
/// version = "0.1.0"
/// authors = ["Alice"]
/// license = "MIT"
/// category = "review"
/// "#;
///
/// let manifest = TemplateManifest::parse(toml)?;
/// assert_eq!(manifest.template.name, "code-review");
/// assert_eq!(manifest.template.category, Some("review".to_string()));
/// # Ok::<(), ironflow_templates::error::TemplateError>(())
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateManifest {
    /// The `[template]` section.
    pub template: TemplateMetadata,
    /// The `[dependencies]` section (optional).
    #[serde(default)]
    pub dependencies: HashMap<String, DependencySpec>,
}

/// Metadata about a single template.
#[derive(Debug, Clone, Deserialize)]
pub struct TemplateMetadata {
    /// Template name (used as folder name when installing).
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Semver version string.
    pub version: String,
    /// Template authors.
    #[serde(default)]
    pub authors: Vec<String>,
    /// SPDX license identifier.
    #[serde(default)]
    pub license: Option<String>,
    /// Category path for UI grouping (e.g. `"ci-cd"`).
    #[serde(default)]
    pub category: Option<String>,
}

/// A Cargo dependency specification.
///
/// Supports both simple version strings (`"1.0"`) and table form
/// with features (`{ version = "1", features = ["derive"] }`).
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// Simple version string, e.g. `"1.0"`.
    Simple(String),
    /// Table form with optional features.
    Detailed(DetailedDependency),
}

/// Detailed dependency with version and optional features.
#[derive(Debug, Clone, Deserialize)]
pub struct DetailedDependency {
    /// Crate version requirement.
    pub version: String,
    /// Optional feature flags.
    #[serde(default)]
    pub features: Vec<String>,
}

impl TemplateManifest {
    /// Parse a manifest from a TOML string.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::InvalidManifest`] if the TOML is malformed
    /// or required fields are missing.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_templates::manifest::TemplateManifest;
    ///
    /// let toml = r#"
    /// [template]
    /// name = "changelog"
    /// description = "Generate changelogs from git history"
    /// version = "1.0.0"
    ///
    /// [dependencies]
    /// serde = { version = "1", features = ["derive"] }
    /// "#;
    ///
    /// let manifest = TemplateManifest::parse(toml)?;
    /// assert_eq!(manifest.template.name, "changelog");
    /// assert_eq!(manifest.dependencies.len(), 1);
    /// # Ok::<(), ironflow_templates::error::TemplateError>(())
    /// ```
    pub fn parse(toml_str: &str) -> Result<Self, TemplateError> {
        toml::from_str(toml_str).map_err(|e| TemplateError::InvalidManifest(e.to_string()))
    }

    /// Read and parse a manifest from a file path.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::Io`] if the file cannot be read, or
    /// [`TemplateError::InvalidManifest`] if parsing fails.
    pub fn from_file(path: &Path) -> Result<Self, TemplateError> {
        let content = fs::read_to_string(path).map_err(TemplateError::Io)?;
        Self::parse(&content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_manifest() {
        let toml = r#"
[template]
name = "ci-pipeline"
description = "Generic CI pipeline with build, test, deploy"
version = "1.0.0"
authors = ["Alice", "Bob"]
license = "MIT"
category = "ci-cd"

[dependencies]
ironflow-engine = "0.1"
serde = { version = "1", features = ["derive"] }
"#;

        let manifest = TemplateManifest::parse(toml).unwrap();
        assert_eq!(manifest.template.name, "ci-pipeline");
        assert_eq!(
            manifest.template.description,
            "Generic CI pipeline with build, test, deploy"
        );
        assert_eq!(manifest.template.version, "1.0.0");
        assert_eq!(manifest.template.authors, vec!["Alice", "Bob"]);
        assert_eq!(manifest.template.license, Some("MIT".to_string()));
        assert_eq!(manifest.template.category, Some("ci-cd".to_string()));
        assert_eq!(manifest.dependencies.len(), 2);
    }

    #[test]
    fn parse_missing_name() {
        let toml = r#"
[template]
description = "No name"
version = "1.0.0"
"#;

        let result = TemplateManifest::parse(toml);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("name"),
            "error should mention missing 'name' field: {err}"
        );
    }

    #[test]
    fn parse_deps_with_features() {
        let toml = r#"
[template]
name = "test"
description = "test"
version = "0.1.0"

[dependencies]
serde = { version = "1", features = ["derive", "rc"] }
tokio = { version = "1", features = ["full"] }
simple-dep = "0.5"
"#;

        let manifest = TemplateManifest::parse(toml).unwrap();
        assert_eq!(manifest.dependencies.len(), 3);

        match &manifest.dependencies["serde"] {
            DependencySpec::Detailed(d) => {
                assert_eq!(d.version, "1");
                assert_eq!(d.features, vec!["derive", "rc"]);
            }
            DependencySpec::Simple(_) => panic!("expected detailed dependency for serde"),
        }

        match &manifest.dependencies["simple-dep"] {
            DependencySpec::Simple(v) => assert_eq!(v, "0.5"),
            DependencySpec::Detailed(_) => panic!("expected simple dependency for simple-dep"),
        }
    }

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
[template]
name = "minimal"
description = "Minimal template"
version = "0.1.0"
"#;

        let manifest = TemplateManifest::parse(toml).unwrap();
        assert_eq!(manifest.template.name, "minimal");
        assert!(manifest.template.authors.is_empty());
        assert_eq!(manifest.template.license, None);
        assert_eq!(manifest.template.category, None);
        assert!(manifest.dependencies.is_empty());
    }

    #[test]
    fn parse_invalid_toml() {
        let result = TemplateManifest::parse("not valid toml [[[");
        assert!(result.is_err());
    }
}
