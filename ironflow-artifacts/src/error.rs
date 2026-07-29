//! Error type for blob storage operations.

use thiserror::Error;

/// Errors produced by [`BlobStore`](crate::blob_store::BlobStore) operations.
///
/// # Examples
///
/// ```
/// use ironflow_artifacts::error::ArtifactError;
///
/// let err = ArtifactError::NotFound("artifacts/a/b/c".to_string());
/// assert!(err.to_string().contains("not found"));
/// ```
#[derive(Debug, Error)]
pub enum ArtifactError {
    /// No blob is stored under this key.
    #[error("artifact not found: {0}")]
    NotFound(String),

    /// The artifact name violates the naming rules.
    ///
    /// See [`validate_artifact_name`](crate::name::validate_artifact_name).
    #[error("invalid artifact name {name:?}: {reason}")]
    InvalidName {
        /// The rejected name.
        name: String,
        /// Why it was rejected.
        reason: &'static str,
    },

    /// The storage key is not usable by this backend.
    ///
    /// Keys are generated from UUIDs, so this signals a programming error
    /// rather than bad user input.
    #[error("invalid storage key {key:?}: {reason}")]
    InvalidKey {
        /// The rejected key.
        key: String,
        /// Why it was rejected.
        reason: &'static str,
    },

    /// The payload exceeded the configured size limit.
    #[error("artifact exceeds the {limit_bytes} byte limit")]
    TooLarge {
        /// The configured limit, in bytes.
        limit_bytes: u64,
    },

    /// An I/O error from the backing storage.
    #[error("artifact storage io error: {0}")]
    Io(String),
}

impl From<std::io::Error> for ArtifactError {
    fn from(err: std::io::Error) -> Self {
        ArtifactError::Io(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Error as IoError, ErrorKind};

    use super::*;

    #[test]
    fn not_found_display() {
        let err = ArtifactError::NotFound("a/b".to_string());
        assert_eq!(err.to_string(), "artifact not found: a/b");
    }

    #[test]
    fn invalid_name_display_quotes_the_name() {
        let err = ArtifactError::InvalidName {
            name: "../etc/passwd".to_string(),
            reason: "contains a forbidden character",
        };
        assert!(err.to_string().contains("\"../etc/passwd\""));
        assert!(err.to_string().contains("forbidden character"));
    }

    #[test]
    fn invalid_key_display() {
        let err = ArtifactError::InvalidKey {
            key: "/abs".to_string(),
            reason: "must be relative",
        };
        assert!(err.to_string().contains("must be relative"));
    }

    #[test]
    fn too_large_display_shows_the_limit() {
        let err = ArtifactError::TooLarge { limit_bytes: 1024 };
        assert!(err.to_string().contains("1024"));
    }

    #[test]
    fn io_error_converts() {
        let io = IoError::new(ErrorKind::PermissionDenied, "nope");
        let err = ArtifactError::from(io);
        assert!(matches!(err, ArtifactError::Io(_)));
        assert!(err.to_string().contains("nope"));
    }
}
