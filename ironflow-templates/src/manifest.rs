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

use semver::Version;
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
    /// The `[requirements]` section (optional): what the installer must
    /// provide outside of the copied source files.
    #[serde(default)]
    pub requirements: Requirements,
}

/// What a template needs from the project that installs it, beyond Rust
/// dependencies. Printed after `ironflow-cli template add`.
///
/// # Examples
///
/// ```
/// use ironflow_templates::manifest::TemplateManifest;
///
/// let manifest = TemplateManifest::parse(r#"
/// [template]
/// name = "gitlab-mr-review"
/// description = "Review merge requests"
/// version = "0.1.0"
///
/// [requirements]
/// tools = ["git"]
/// secrets = ["gitlab_token"]
/// "#)?;
/// assert_eq!(manifest.requirements.tools, vec!["git"]);
/// assert!(manifest.requirements.render().contains("gitlab_token"));
/// # Ok::<(), ironflow_templates::error::TemplateError>(())
/// ```
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Requirements {
    /// Environment variables the installer must set.
    #[serde(default)]
    pub env: Vec<String>,
    /// Workflow secrets (Ironflow secret store) the handler reads.
    #[serde(default)]
    pub secrets: Vec<String>,
    /// Executables expected on the worker or in the agent runner image.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Free-form setup steps that fit none of the lists above.
    #[serde(default)]
    pub notes: Vec<String>,
}

impl Requirements {
    /// `true` when the template declares no requirement at all.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_templates::manifest::Requirements;
    ///
    /// assert!(Requirements::default().is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.env.is_empty()
            && self.secrets.is_empty()
            && self.tools.is_empty()
            && self.notes.is_empty()
    }

    /// Render the requirements as an indented, human-readable block, or an
    /// empty string when there are none. Empty groups are left out.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_templates::manifest::Requirements;
    ///
    /// let requirements = Requirements {
    ///     tools: vec!["git".to_string()],
    ///     ..Requirements::default()
    /// };
    /// assert_eq!(
    ///     requirements.render(),
    ///     "Requirements:\n  tools in the runner image:\n    - git\n",
    /// );
    /// ```
    pub fn render(&self) -> String {
        if self.is_empty() {
            return String::new();
        }

        let groups = [
            ("environment variables", &self.env),
            ("workflow secrets", &self.secrets),
            ("tools in the runner image", &self.tools),
            ("notes", &self.notes),
        ];
        let mut out = String::from("Requirements:\n");
        for (title, items) in groups {
            if items.is_empty() {
                continue;
            }
            out.push_str(&format!("  {title}:\n"));
            for item in items {
                out.push_str(&format!("    - {item}\n"));
            }
        }
        out
    }
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
    /// Minimum Ironflow engine version required by this template.
    #[serde(default)]
    pub min_ironflow_version: Option<Version>,
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

/// Validate that a template name is safe for use in file paths and code.
///
/// Accepts only lowercase ASCII letters, digits, and hyphens. Rejects
/// empty strings, leading/trailing hyphens, and names containing path
/// traversal characters.
///
/// # Errors
///
/// Returns [`TemplateError::InvalidManifest`] if the name is rejected.
///
/// # Examples
///
/// ```
/// use ironflow_templates::manifest::validate_template_name;
///
/// assert!(validate_template_name("ci-pipeline").is_ok());
/// assert!(validate_template_name("hello123").is_ok());
/// assert!(validate_template_name("../evil").is_err());
/// assert!(validate_template_name("").is_err());
/// ```
pub fn validate_template_name(name: &str) -> Result<(), TemplateError> {
    if name.is_empty() {
        return Err(TemplateError::InvalidManifest(
            "template name cannot be empty".to_string(),
        ));
    }

    if name.contains("..") || name.contains('/') || name.contains('\\') {
        return Err(TemplateError::InvalidManifest(format!(
            "template name contains forbidden characters: {name}"
        )));
    }

    let valid = name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');

    if !valid {
        return Err(TemplateError::InvalidManifest(format!(
            "template name must contain only lowercase ASCII, digits, and hyphens: {name}"
        )));
    }

    if name.starts_with('-') || name.ends_with('-') {
        return Err(TemplateError::InvalidManifest(format!(
            "template name must not start or end with a hyphen: {name}"
        )));
    }

    Ok(())
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

    #[test]
    fn parse_min_ironflow_version() {
        let toml = r#"
[template]
name = "versioned"
description = "Has min version"
version = "1.0.0"
min_ironflow_version = "0.5.0"
"#;

        let manifest = TemplateManifest::parse(toml).unwrap();
        assert_eq!(
            manifest.template.min_ironflow_version,
            Some(Version::parse("0.5.0").unwrap())
        );
    }

    // ---- template name validation ----

    #[test]
    fn validate_name_accepts_valid() {
        validate_template_name("ci-pipeline").unwrap();
        validate_template_name("hello123").unwrap();
        validate_template_name("a").unwrap();
    }

    #[test]
    fn validate_name_rejects_path_traversal() {
        assert!(validate_template_name("../evil").is_err());
        assert!(validate_template_name("foo/bar").is_err());
        assert!(validate_template_name("foo\\bar").is_err());
    }

    #[test]
    fn validate_name_rejects_empty() {
        assert!(validate_template_name("").is_err());
    }

    #[test]
    fn validate_name_rejects_uppercase() {
        assert!(validate_template_name("MyTemplate").is_err());
    }

    #[test]
    fn validate_name_rejects_leading_hyphen() {
        assert!(validate_template_name("-bad").is_err());
    }

    #[test]
    fn validate_name_rejects_trailing_hyphen() {
        assert!(validate_template_name("bad-").is_err());
    }

    #[test]
    fn parse_requirements() {
        let toml = r#"
[template]
name = "gitlab-mr-review"
description = "Review"
version = "0.1.0"

[requirements]
env = ["GITLAB_WEBHOOK_SECRET"]
secrets = ["gitlab_token"]
tools = ["git"]
notes = ["Mount the workspace volume read-only in the agent pod"]
"#;

        let manifest = TemplateManifest::parse(toml).unwrap();
        let requirements = &manifest.requirements;
        assert_eq!(requirements.env, vec!["GITLAB_WEBHOOK_SECRET"]);
        assert_eq!(requirements.secrets, vec!["gitlab_token"]);
        assert_eq!(requirements.tools, vec!["git"]);
        assert_eq!(
            requirements.notes,
            vec!["Mount the workspace volume read-only in the agent pod"]
        );
        assert!(!requirements.is_empty());
    }

    #[test]
    fn parse_without_requirements_is_empty() {
        let toml = r#"
[template]
name = "plain"
description = "No requirements"
version = "0.1.0"
"#;

        let manifest = TemplateManifest::parse(toml).unwrap();
        assert!(manifest.requirements.is_empty());
        assert!(manifest.requirements.render().is_empty());
    }

    #[test]
    fn parse_rejects_requirements_with_wrong_type() {
        let toml = r#"
[template]
name = "bad"
description = "Bad requirements"
version = "0.1.0"

[requirements]
tools = "git"
"#;

        let err = TemplateManifest::parse(toml).unwrap_err();
        assert!(err.to_string().contains("tools"), "got: {err}");
    }

    #[test]
    fn render_requirements_lists_every_group() {
        let requirements = Requirements {
            env: vec!["GITLAB_WEBHOOK_SECRET".to_string()],
            secrets: vec!["gitlab_token".to_string()],
            tools: vec!["git".to_string()],
            notes: vec!["Run the agent in a sandboxed pod".to_string()],
        };

        assert_eq!(
            requirements.render(),
            "Requirements:\n\
             \x20 environment variables:\n\
             \x20   - GITLAB_WEBHOOK_SECRET\n\
             \x20 workflow secrets:\n\
             \x20   - gitlab_token\n\
             \x20 tools in the runner image:\n\
             \x20   - git\n\
             \x20 notes:\n\
             \x20   - Run the agent in a sandboxed pod\n"
        );
    }

    #[test]
    fn render_requirements_skips_empty_groups() {
        let requirements = Requirements {
            tools: vec!["git".to_string()],
            ..Requirements::default()
        };

        assert_eq!(
            requirements.render(),
            "Requirements:\n  tools in the runner image:\n    - git\n"
        );
    }

    #[test]
    fn parse_without_min_ironflow_version() {
        let toml = r#"
[template]
name = "no-min"
description = "No min version"
version = "1.0.0"
"#;

        let manifest = TemplateManifest::parse(toml).unwrap();
        assert!(manifest.template.min_ironflow_version.is_none());
    }
}
