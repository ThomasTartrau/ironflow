//! [`GitRepo`] -- central handle wrapping a repository path.

use std::path::{Path, PathBuf};

use git2::Repository;
use ironflow_core::error::OperationError;

use crate::helpers::git_error;

/// A handle to a local Git repository, identified by its working directory path.
///
/// `GitRepo` does not hold a [`git2::Repository`] directly because `Repository`
/// is `!Sync` while [`Operation`](ironflow_core::operation::Operation) requires
/// `Send + Sync`. Instead, each operation re-opens the repository inside
/// [`spawn_blocking`](tokio::task::spawn_blocking).
///
/// # Examples
///
/// ```no_run
/// use ironflow_ops_git::GitRepo;
///
/// # fn example() -> Result<(), ironflow_core::error::OperationError> {
/// let repo = GitRepo::open("/path/to/repo")?;
/// assert!(repo.path().exists());
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone)]
pub struct GitRepo {
    path: PathBuf,
}

impl GitRepo {
    /// Open an existing repository at the given path.
    ///
    /// The path should point to the working directory (not `.git/`).
    ///
    /// # Errors
    ///
    /// Returns [`OperationError::External`] if the path is not a valid
    /// Git repository.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use ironflow_ops_git::GitRepo;
    ///
    /// # fn example() -> Result<(), ironflow_core::error::OperationError> {
    /// let repo = GitRepo::open("/path/to/repo")?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn open(path: impl AsRef<Path>) -> Result<Self, OperationError> {
        let path = path.as_ref().to_path_buf();
        let repo = Repository::open(&path).map_err(git_error)?;
        let workdir = repo
            .workdir()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| path.clone());
        Ok(Self { path: workdir })
    }

    /// Create a `GitRepo` from a path without validation.
    ///
    /// Use this when you know the path is valid (e.g. after [`RepoInit`](crate::repository::RepoInit)
    /// or [`RepoClone`](crate::repository::RepoClone)).
    pub fn from_path(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// The working directory path of this repository.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_valid_repo() {
        let tmp = tempfile::tempdir().unwrap();
        Repository::init(tmp.path()).unwrap();
        let repo = GitRepo::open(tmp.path()).unwrap();
        assert!(repo.path().exists());
        assert!(repo.path().join(".git").exists());
    }

    #[test]
    fn open_invalid_path() {
        let err = GitRepo::open("/nonexistent/path/to/repo").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("git error"), "got: {msg}");
    }
}
