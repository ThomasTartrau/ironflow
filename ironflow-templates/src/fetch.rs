//! Fetch template repositories from Git.
//!
//! Clones a remote Git repository into a temporary directory so its
//! templates can be discovered and installed.
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

use crate::error::TemplateError;

/// Clone a Git repository into a temporary directory.
///
/// The returned [`TempDir`] owns the cloned files; they are cleaned up
/// when the value is dropped.
///
/// # Errors
///
/// Returns [`TemplateError::Git`] if the `git clone` command fails.
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
    let tmp = TempDir::new().map_err(TemplateError::Io)?;

    let output = Command::new("git")
        .args(["clone", "--depth", "1", url])
        .arg(tmp.path())
        .output()
        .map_err(|e| TemplateError::Git(format!("failed to run git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(TemplateError::Git(format!("git clone failed: {stderr}")));
    }

    Ok(tmp)
}
