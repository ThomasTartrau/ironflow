//! Artifact name validation and storage key derivation.
//!
//! An artifact name is user-supplied and ends up in a download URL, so it is
//! validated against a strict whitelist. The storage key is derived from UUIDs
//! only and never embeds the name: even a validation bug cannot turn into a
//! path traversal on the storage backend.

use uuid::Uuid;

use crate::error::ArtifactError;

/// Maximum length of an artifact name, in bytes.
pub const MAX_ARTIFACT_NAME_LEN: usize = 255;

/// Prefix under which every artifact blob is stored.
pub const STORAGE_PREFIX: &str = "artifacts";

/// Validate a user-supplied artifact name.
///
/// A valid name matches `^[A-Za-z0-9._][A-Za-z0-9._-]{0,254}$` and is neither
/// `.` nor `..`. Path separators are rejected because they are not in the
/// character class.
///
/// # Errors
///
/// Returns [`ArtifactError::InvalidName`] when the name is empty, longer than
/// [`MAX_ARTIFACT_NAME_LEN`], starts with `-`, contains a character outside the
/// whitelist, or is a relative path component.
///
/// # Examples
///
/// ```
/// use ironflow_artifacts::name::validate_artifact_name;
///
/// # fn main() -> Result<(), ironflow_artifacts::error::ArtifactError> {
/// validate_artifact_name("report.html")?;
/// assert!(validate_artifact_name("../etc/passwd").is_err());
/// # Ok(())
/// # }
/// ```
pub fn validate_artifact_name(name: &str) -> Result<(), ArtifactError> {
    let reject = |reason: &'static str| {
        Err(ArtifactError::InvalidName {
            name: name.to_string(),
            reason,
        })
    };

    if name.is_empty() {
        return reject("must not be empty");
    }
    if name.len() > MAX_ARTIFACT_NAME_LEN {
        return reject("must be at most 255 bytes");
    }
    if name == "." || name == ".." {
        return reject("must not be a relative path component");
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    {
        return reject("may only contain ASCII letters, digits, '.', '_' and '-'");
    }
    // A leading '-' makes the name look like a flag to any CLI that consumes it.
    if name.starts_with('-') {
        return reject("must not start with '-'");
    }

    Ok(())
}

/// Derive the storage key for an artifact blob.
///
/// The key is built exclusively from UUIDs, so it never carries user input.
///
/// # Examples
///
/// ```
/// use ironflow_artifacts::name::storage_key;
/// use uuid::Uuid;
///
/// let key = storage_key(Uuid::nil(), Uuid::nil(), Uuid::nil());
/// assert!(key.starts_with("artifacts/"));
/// ```
pub fn storage_key(run_id: Uuid, step_id: Uuid, artifact_id: Uuid) -> String {
    format!("{STORAGE_PREFIX}/{run_id}/{step_id}/{artifact_id}")
}

/// Guess the MIME type of an artifact from its name.
///
/// Falls back to `application/octet-stream` when the extension is unknown or
/// absent. Callers may override the result with an explicit content type.
///
/// # Examples
///
/// ```
/// use ironflow_artifacts::name::guess_content_type;
///
/// assert_eq!(guess_content_type("report.html"), "text/html");
/// assert_eq!(guess_content_type("build.bin"), "application/octet-stream");
/// ```
pub fn guess_content_type(name: &str) -> String {
    mime_guess::from_path(name)
        .first_raw()
        .unwrap_or("application/octet-stream")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_plain_file_name() {
        assert!(validate_artifact_name("report.html").is_ok());
    }

    #[test]
    fn accepts_dots_underscores_and_dashes() {
        assert!(validate_artifact_name("my_build-2.tar.gz").is_ok());
    }

    #[test]
    fn accepts_a_leading_dot() {
        assert!(validate_artifact_name(".gitignore").is_ok());
    }

    #[test]
    fn rejects_empty() {
        assert!(validate_artifact_name("").is_err());
    }

    #[test]
    fn rejects_path_traversal() {
        assert!(validate_artifact_name("../../etc/passwd").is_err());
        assert!(validate_artifact_name("..").is_err());
        assert!(validate_artifact_name(".").is_err());
    }

    #[test]
    fn rejects_path_separators() {
        assert!(validate_artifact_name("dir/file.txt").is_err());
        assert!(validate_artifact_name("dir\\file.txt").is_err());
    }

    #[test]
    fn rejects_a_leading_dash() {
        assert!(validate_artifact_name("-rf").is_err());
    }

    #[test]
    fn rejects_unicode() {
        assert!(validate_artifact_name("rapport-été.html").is_err());
    }

    #[test]
    fn rejects_null_and_control_characters() {
        assert!(validate_artifact_name("a\0b").is_err());
        assert!(validate_artifact_name("a\nb").is_err());
    }

    #[test]
    fn accepts_exactly_255_bytes_and_rejects_256() {
        let ok = "a".repeat(MAX_ARTIFACT_NAME_LEN);
        assert!(validate_artifact_name(&ok).is_ok());

        let too_long = "a".repeat(MAX_ARTIFACT_NAME_LEN + 1);
        assert!(validate_artifact_name(&too_long).is_err());
    }

    #[test]
    fn storage_key_contains_only_uuids() {
        let run = Uuid::now_v7();
        let step = Uuid::now_v7();
        let artifact = Uuid::now_v7();

        let key = storage_key(run, step, artifact);

        assert_eq!(key, format!("artifacts/{run}/{step}/{artifact}"));
        assert!(!key.contains(".."));
    }

    #[test]
    fn guesses_common_types() {
        assert_eq!(guess_content_type("a.json"), "application/json");
        assert_eq!(guess_content_type("a.txt"), "text/plain");
        assert_eq!(guess_content_type("a.png"), "image/png");
    }

    #[test]
    fn guesses_octet_stream_without_extension() {
        assert_eq!(guess_content_type("build"), "application/octet-stream");
    }
}
