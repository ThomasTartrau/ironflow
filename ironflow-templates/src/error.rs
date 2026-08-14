//! Error types for the template system.

use std::io;

use thiserror::Error;

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
        /// Comma-separated list of available template names.
        available: String,
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
}
