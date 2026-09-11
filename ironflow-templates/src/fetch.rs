//! Fetch template repositories from Git.
//!
//! Clones a remote Git repository into a temporary directory so its
//! templates can be discovered and installed. Supports cloning at a
//! specific tag or resolving the latest semver tag.
//!
//! # Examples
//!
//! ```no_run
//! use ironflow_templates::fetch::fetch_repo;
//!
//! # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
//! let tmp = fetch_repo("https://github.com/user/templates.git")?;
//! // `tmp.path()` contains the cloned repo
//! # Ok(())
//! # }
//! ```

use std::process::Command;

use tempfile::TempDir;
use url::Url;

use crate::error::TemplateError;

/// Validate that a URL looks like a legitimate Git remote.
///
/// Rejects values that could be interpreted as Git flags (starting with `-`)
/// and requires a recognized scheme (`https://`, `http://`, `git://`,
/// `ssh://`) or SCP-style syntax (`git@host:path`).
///
/// # Errors
///
/// Returns [`TemplateError::Git`] if the URL is rejected.
///
/// # Examples
///
/// ```
/// use ironflow_templates::fetch::validate_git_url;
///
/// assert!(validate_git_url("https://github.com/user/repo").is_ok());
/// assert!(validate_git_url("--upload-pack=evil").is_err());
/// assert!(validate_git_url("file:///tmp/local-repo").is_ok());
/// ```
pub fn validate_git_url(input: &str) -> Result<(), TemplateError> {
    if input.starts_with('-') {
        return Err(TemplateError::Git(
            "URL must not start with '-'".to_string(),
        ));
    }

    // SCP-style syntax (git@host:path) is not a valid URL but is a
    // legitimate Git remote; url::Url cannot parse it.
    let is_scp = input.contains('@') && input.contains(':');
    if is_scp {
        return Ok(());
    }

    let parsed = Url::parse(input).map_err(|e| TemplateError::Git(format!("invalid URL: {e}")))?;

    match parsed.scheme() {
        "https" | "http" | "git" | "ssh" | "file" => Ok(()),
        other => Err(TemplateError::Git(format!(
            "unsupported URL scheme: {other}"
        ))),
    }
}

/// Clone a Git repository into a temporary directory.
///
/// The returned [`TempDir`] owns the cloned files; they are cleaned up
/// when the value is dropped.
///
/// # Errors
///
/// Returns [`TemplateError::Git`] if the `git clone` command fails or
/// the URL is invalid.
///
/// # Examples
///
/// ```no_run
/// use ironflow_templates::fetch::fetch_repo;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let tmp = fetch_repo("https://github.com/user/templates.git")?;
/// println!("cloned to {}", tmp.path().display());
/// # Ok(())
/// # }
/// ```
pub fn fetch_repo(url: &str) -> Result<TempDir, TemplateError> {
    validate_git_url(url)?;
    clone_repo(url, None)
}

/// Clone a Git repository at a specific tag into a temporary directory.
///
/// Uses `git clone --depth 1 --branch <tag>` for an efficient shallow clone
/// of exactly the tagged revision.
///
/// # Errors
///
/// Returns [`TemplateError::Git`] if the clone fails (e.g. the tag does
/// not exist) or the URL is invalid.
///
/// # Examples
///
/// ```no_run
/// use ironflow_templates::fetch::fetch_repo_at_tag;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let tmp = fetch_repo_at_tag("https://github.com/user/templates.git", "v1.2.0")?;
/// # Ok(())
/// # }
/// ```
pub fn fetch_repo_at_tag(url: &str, tag: &str) -> Result<TempDir, TemplateError> {
    validate_git_url(url)?;
    clone_repo(url, Some(tag))
}

/// Internal clone helper. Uses `--` to separate options from the URL.
fn clone_repo(url: &str, branch: Option<&str>) -> Result<TempDir, TemplateError> {
    let tmp = TempDir::new().map_err(TemplateError::Io)?;

    let mut cmd = Command::new("git");
    cmd.args(["clone", "--depth", "1"]);
    if let Some(tag) = branch {
        cmd.args(["--branch", tag]);
    }
    cmd.arg("--").arg(url).arg(tmp.path());

    let output = cmd
        .output()
        .map_err(|e| TemplateError::Git(format!("failed to run git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let context = branch.map(|t| format!(" at tag '{t}'")).unwrap_or_default();
        return Err(TemplateError::Git(format!(
            "git clone{context} failed: {stderr}"
        )));
    }

    Ok(tmp)
}

/// List remote tags from a Git repository and return the latest semver tag.
///
/// Runs `git ls-remote --tags <url>`, parses the tag names, filters for
/// valid semver tags (with or without `v` prefix), and returns the highest
/// version tag name (including its `v` prefix if present).
///
/// # Errors
///
/// Returns [`TemplateError::NoTagsFound`] if the repository has no
/// semver-parseable tags, or [`TemplateError::Git`] if `git ls-remote`
/// fails.
///
/// # Examples
///
/// ```no_run
/// use ironflow_templates::fetch::fetch_latest_tag;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let tag = fetch_latest_tag("https://github.com/user/templates.git")?;
/// println!("latest tag: {tag}");
/// # Ok(())
/// # }
/// ```
pub fn fetch_latest_tag(url: &str) -> Result<String, TemplateError> {
    validate_git_url(url)?;

    let output = Command::new("git")
        .args(["ls-remote", "--tags", "--", url])
        .output()
        .map_err(|e| TemplateError::Git(format!("failed to run git ls-remote: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TemplateError::Git(format!(
            "git ls-remote --tags failed: {stderr}"
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_latest_tag(&stdout, url)
}

/// Parse the latest semver tag from `git ls-remote --tags` output.
fn parse_latest_tag(ls_remote_output: &str, url: &str) -> Result<String, TemplateError> {
    let mut best: Option<(semver::Version, String)> = None;

    for line in ls_remote_output.lines() {
        let Some(ref_name) = line.split('\t').nth(1) else {
            continue;
        };

        if ref_name.ends_with("^{}") {
            continue;
        }

        let tag_name = ref_name.strip_prefix("refs/tags/").unwrap_or(ref_name);
        let version_str = tag_name.strip_prefix('v').unwrap_or(tag_name);

        if let Ok(version) = semver::Version::parse(version_str)
            && best.as_ref().is_none_or(|(b, _)| version > *b)
        {
            best = Some((version, tag_name.to_string()));
        }
    }

    best.map(|(_, tag)| tag)
        .ok_or_else(|| TemplateError::NoTagsFound(url.to_string()))
}

pub use crate::manifest::validate_template_name;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_latest_tag_finds_highest_semver() {
        let output = "\
abc123\trefs/tags/v1.0.0\n\
def456\trefs/tags/v1.2.0\n\
ghi789\trefs/tags/v0.9.0\n\
jkl012\trefs/tags/v1.2.0^{}\n";

        let tag = parse_latest_tag(output, "https://example.com").unwrap();
        assert_eq!(tag, "v1.2.0");
    }

    #[test]
    fn parse_latest_tag_without_v_prefix() {
        let output = "\
abc123\trefs/tags/1.0.0\n\
def456\trefs/tags/2.0.0\n";

        let tag = parse_latest_tag(output, "https://example.com").unwrap();
        assert_eq!(tag, "2.0.0");
    }

    #[test]
    fn parse_latest_tag_mixed_prefixes() {
        let output = "\
abc123\trefs/tags/v1.0.0\n\
def456\trefs/tags/3.0.0\n\
ghi789\trefs/tags/v2.0.0\n";

        let tag = parse_latest_tag(output, "https://example.com").unwrap();
        assert_eq!(tag, "3.0.0");
    }

    #[test]
    fn parse_latest_tag_no_valid_tags() {
        let output = "\
abc123\trefs/tags/not-semver\n\
def456\trefs/tags/release-candidate\n";

        let err = parse_latest_tag(output, "https://example.com/repo").unwrap_err();
        assert!(err.to_string().contains("no tags found"));
    }

    #[test]
    fn parse_latest_tag_empty_output() {
        let err = parse_latest_tag("", "https://example.com/repo").unwrap_err();
        assert!(err.to_string().contains("no tags found"));
    }

    #[test]
    fn parse_latest_tag_skips_dereferenced() {
        let output = "\
abc123\trefs/tags/v1.0.0\n\
def456\trefs/tags/v2.0.0^{}\n";

        let tag = parse_latest_tag(output, "https://example.com").unwrap();
        assert_eq!(tag, "v1.0.0");
    }

    #[test]
    fn parse_latest_tag_prerelease_sorted_correctly() {
        let output = "\
abc123\trefs/tags/v1.0.0-alpha\n\
def456\trefs/tags/v1.0.0\n";

        let tag = parse_latest_tag(output, "https://example.com").unwrap();
        assert_eq!(tag, "v1.0.0");
    }

    // ---- URL validation ----

    #[test]
    fn validate_git_url_accepts_https() {
        validate_git_url("https://github.com/user/repo").unwrap();
    }

    #[test]
    fn validate_git_url_accepts_ssh() {
        validate_git_url("ssh://git@github.com/user/repo").unwrap();
    }

    #[test]
    fn validate_git_url_accepts_scp() {
        validate_git_url("git@github.com:user/repo.git").unwrap();
    }

    #[test]
    fn validate_git_url_rejects_flag_injection() {
        let err = validate_git_url("--upload-pack=evil").unwrap_err();
        assert!(err.to_string().contains("must not start with '-'"));
    }

    #[test]
    fn validate_git_url_rejects_unknown_scheme() {
        let err = validate_git_url("ftp://example.com/repo").unwrap_err();
        assert!(err.to_string().contains("unsupported URL scheme"));
    }

    #[test]
    fn validate_git_url_accepts_file_scheme() {
        validate_git_url("file:///tmp/local-repo").unwrap();
    }

    #[test]
    fn validate_git_url_rejects_bare_path() {
        let err = validate_git_url("/tmp/local-repo").unwrap_err();
        assert!(err.to_string().contains("invalid URL"));
    }
}
