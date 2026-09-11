//! Error types for the template system.

use std::fmt;
use std::io;

use semver::Version;
use thiserror::Error;

/// A list of names formatted as a comma-separated string in error messages.
#[derive(Debug)]
pub struct NameList(pub Vec<String>);

impl fmt::Display for NameList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.join(", "))
    }
}

/// Errors that can occur during template operations.
#[derive(Debug, Error)]
pub enum TemplateError {
    /// The `template.toml` manifest is malformed or missing required fields.
    #[error("invalid template manifest: {0}")]
    InvalidManifest(String),

    /// A filesystem operation failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// No templates found in the given directory.
    #[error("no templates found in {0}")]
    NoTemplatesFound(String),

    /// The requested template was not found.
    #[error("template '{name}' not found, available: {available}")]
    TemplateNotFound {
        /// The name that was looked up.
        name: String,
        /// Available template names.
        available: NameList,
    },

    /// The destination directory already exists.
    #[error("destination already exists: {0}")]
    AlreadyInstalled(String),

    /// Two template directories declare the same name.
    #[error("duplicate template name '{name}': found in '{first}' and '{second}'")]
    DuplicateName {
        /// The conflicting template name.
        name: String,
        /// Path to the first directory.
        first: String,
        /// Path to the second directory.
        second: String,
    },

    /// A git operation failed.
    #[error("git error: {0}")]
    Git(String),

    /// The template requires a newer version of Ironflow than the project has.
    #[error("template requires Ironflow >= {template_min}, project has {project_version}")]
    VersionIncompatible {
        /// Minimum version the template declares.
        template_min: Version,
        /// Version found in the project's `Cargo.toml`.
        project_version: Version,
    },

    /// Failed to fetch or parse the remote registry index.
    #[error("registry error: {0}")]
    Registry(String),

    /// No Git tags found in the repository.
    #[error("no tags found in repository: {0}")]
    NoTagsFound(String),

    /// A lockfile operation failed.
    #[error("lockfile error: {0}")]
    Lockfile(String),

    /// The template was not found in the registry index.
    #[error("template '{name}' not found in registry, available: {available}")]
    NotInRegistry {
        /// The name that was looked up.
        name: String,
        /// Available names in the registry.
        available: NameList,
    },

    /// A semver parsing error.
    #[error("invalid version: {0}")]
    InvalidVersion(String),
}
