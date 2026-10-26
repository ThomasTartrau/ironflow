//! Validation of short branch and tag names, without spawning `git`.
//!
//! The rules follow `git check-ref-format`: operations check their names with
//! them before touching the repository, so a name such as `-x` or `a..b` is
//! refused with a clear error instead of reaching libgit2 or the `git` binary.

use ironflow_core::error::OperationError;

use crate::helpers::external_error;

/// Check that `name` is a valid short branch or tag name.
///
/// Applies the rules of `git check-ref-format` without calling `git`. Names
/// such as `feature/x`, `origin/main`, `run-123` and `v1.2.3` are accepted.
///
/// # Examples
///
/// ```
/// use ironflow_core::error::OperationError;
/// use ironflow_ops_git::refname::validate_ref_name;
///
/// # fn main() -> Result<(), OperationError> {
/// validate_ref_name("feature/x")?;
/// assert!(validate_ref_name("a..b").is_err());
/// assert!(validate_ref_name("-x").is_err());
/// # Ok(())
/// # }
/// ```
///
/// # Errors
///
/// Returns [`OperationError::External`] (origin `git`) for an empty name, a
/// name starting with `-`, `@` or `HEAD`, a name holding a control character,
/// a space or one of `~ ^ : ? * [ \`, a name holding `..` or `@{`, a leading,
/// trailing or doubled `/`, a trailing `.`, or a component that starts with
/// `.` or ends with `.lock`.
pub fn validate_ref_name(name: &str) -> Result<(), OperationError> {
    check_ref_name("ref", name)
}

/// Check `name` like [`validate_ref_name`], `kind` naming it in the error.
///
/// # Errors
///
/// Returns [`OperationError::External`] with the message
/// `invalid {kind} name {name:?}: {reason}`.
pub(crate) fn check_ref_name(kind: &str, name: &str) -> Result<(), OperationError> {
    match reject_reason(name) {
        Some(reason) => Err(external_error(format!(
            "invalid {kind} name {name:?}: {reason}"
        ))),
        None => Ok(()),
    }
}

fn reject_reason(name: &str) -> Option<&'static str> {
    if name.is_empty() {
        return Some("empty name");
    }
    if name.starts_with('-') {
        return Some("starts with '-'");
    }
    if name == "@" || name == "HEAD" {
        return Some("reserved name");
    }
    if name.chars().any(|c| c.is_control()) {
        return Some("contains a control character");
    }
    if name
        .chars()
        .any(|c| matches!(c, ' ' | '~' | '^' | ':' | '?' | '*' | '[' | '\\'))
    {
        return Some("contains a forbidden character");
    }
    if name.contains("..") {
        return Some("contains '..'");
    }
    if name.contains("@{") {
        return Some("contains '@{'");
    }
    if name.starts_with('/') || name.ends_with('/') {
        return Some("starts or ends with '/'");
    }
    if name.contains("//") {
        return Some("contains '//'");
    }
    if name.ends_with('.') {
        return Some("ends with '.'");
    }
    if name
        .split('/')
        .any(|part| part.starts_with('.') || part.ends_with(".lock"))
    {
        return Some("a component starts with '.' or ends with '.lock'");
    }
    None
}

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::*;

    #[test]
    fn accepts_ordinary_ref_names() {
        for name in [
            "feature/ok-1",
            "feature/x",
            "origin/main",
            "run-123",
            "v1.2.3",
            "été",
            "main",
        ] {
            check_ref_name("branch", name).unwrap_or_else(|e| panic!("rejected {name:?}: {e}"));
            validate_ref_name(name).unwrap();
        }
    }

    #[test]
    fn rejects_dangerous_ref_names() {
        for name in [
            "",
            "-bad",
            "-x",
            "a..b",
            "a b",
            "x~1",
            "a:b",
            "a/",
            "/a",
            "a//b",
            ".hidden",
            "a/.hidden",
            "x.lock",
            "a/x.lock/b",
            "foo@{1}",
            "HEAD",
            "@",
            "a\nb",
            "a\x7fb",
            "a^b",
            "a?b",
            "a*b",
            "a[b",
            "a\\b",
            "end.",
        ] {
            let err = check_ref_name("branch", name).unwrap_err().to_string();
            assert!(err.contains("invalid branch name"), "{name:?} -> {err}");
        }
    }

    #[test]
    fn rejection_message_names_the_reason() {
        let err = check_ref_name("tag", "a..b").unwrap_err().to_string();
        assert!(err.contains("invalid tag name \"a..b\": "), "got: {err}");
    }

    #[test]
    fn agrees_with_git_check_ref_format() {
        for name in [
            "main",
            "feature/x",
            "v1.2.3",
            "run-123",
            "été",
            "a..b",
            "a b",
            "x~1",
            "a:b",
            "a/",
            "/a",
            "a//b",
            ".hidden",
            "a/.hidden",
            "x.lock",
            "foo@{1}",
            "a^b",
            "a?b",
            "a*b",
            "a[b",
            "a\\b",
            "end.",
        ] {
            // "@" is left out on purpose: `--branch` expands it to the current
            // branch before checking, so git accepts it, yet it cannot name a
            // branch. It is covered by `rejects_dangerous_ref_names`.
            let status = Command::new("git")
                .args(["check-ref-format", "--branch", name])
                .output()
                .unwrap()
                .status
                .success();
            assert_eq!(
                status,
                check_ref_name("branch", name).is_ok(),
                "disagreement on {name:?}"
            );
        }
    }
}
