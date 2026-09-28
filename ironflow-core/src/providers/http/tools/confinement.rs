//! Path confinement shared by the file-system tools.
//!
//! A tool that touches the local file system is given a list of allowed
//! roots. Both the roots and every path the model asks for are resolved to
//! canonical paths (`..`, `.` and symbolic links resolved) before being
//! compared, so neither `root/../elsewhere` nor a symbolic link placed under a
//! root can escape it.

use std::fs::canonicalize;
use std::path::{Path, PathBuf};

use super::tool_trait::ToolError;

/// Canonical directories a file-system tool is allowed to access.
#[derive(Debug, Clone)]
pub(crate) struct AllowedRoots {
    roots: Vec<PathBuf>,
}

impl AllowedRoots {
    /// Canonicalize `roots` once.
    ///
    /// # Errors
    ///
    /// Returns [`ToolError`] if `roots` is empty, or if a root does not exist,
    /// cannot be resolved, or is not a directory.
    pub(crate) fn new(roots: Vec<PathBuf>) -> Result<Self, ToolError> {
        if roots.is_empty() {
            return Err(ToolError::new("at least one allowed root is required"));
        }
        let mut canonical = Vec::with_capacity(roots.len());
        for root in roots {
            let resolved = canonicalize(&root).map_err(|e| {
                ToolError::new(format!(
                    "allowed root '{}' cannot be resolved: {e}",
                    root.display()
                ))
            })?;
            if !resolved.is_dir() {
                return Err(ToolError::new(format!(
                    "allowed root '{}' is not a directory",
                    root.display()
                )));
            }
            canonical.push(resolved);
        }
        Ok(Self { roots: canonical })
    }

    /// The first root, used when the model does not give a path.
    pub(crate) fn first(&self) -> &Path {
        &self.roots[0]
    }

    /// Whether the canonical path `canonical` lies under one of the roots.
    pub(crate) fn contains(&self, canonical: &Path) -> bool {
        self.roots.iter().any(|root| canonical.starts_with(root))
    }

    /// Resolve a path requested by the model to a canonical path under a root.
    ///
    /// # Errors
    ///
    /// Returns a message meant for the model when `requested` is relative,
    /// cannot be resolved, or resolves outside every root. An unresolvable
    /// path gets the same message as an out-of-root one, so the model cannot
    /// probe for the existence of files outside the roots.
    pub(crate) fn resolve(&self, requested: &str) -> Result<PathBuf, String> {
        let path = Path::new(requested);
        if !path.is_absolute() {
            return Err(format!(
                "Access denied: path '{requested}' is relative, use an absolute path under {}",
                self.describe()
            ));
        }
        match canonicalize(path) {
            Ok(resolved) if self.contains(&resolved) => Ok(resolved),
            _ => Err(format!(
                "Access denied: path '{requested}' is outside allowed directories ({})",
                self.describe()
            )),
        }
    }

    /// Like [`resolve`](Self::resolve), but an absent or empty `requested`
    /// gives the first root.
    ///
    /// # Errors
    ///
    /// Same as [`resolve`](Self::resolve).
    pub(crate) fn resolve_or_first(&self, requested: Option<&str>) -> Result<PathBuf, String> {
        match requested {
            None | Some("") => Ok(self.first().to_path_buf()),
            Some(requested) => self.resolve(requested),
        }
    }

    /// Human-readable list of the roots, for tool descriptions and errors.
    pub(crate) fn describe(&self) -> String {
        self.roots
            .iter()
            .map(|root| root.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

#[cfg(test)]
mod tests {
    use std::fs::{create_dir, write};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn empty_roots_is_a_construction_error() {
        let err = AllowedRoots::new(Vec::new()).expect_err("empty roots must fail");
        assert!(err.message.contains("at least one"), "{}", err.message);
    }

    #[test]
    fn missing_root_is_a_construction_error() {
        let dir = tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist");
        let err = AllowedRoots::new(vec![missing]).expect_err("missing root must fail");
        assert!(err.message.contains("does-not-exist"), "{}", err.message);
    }

    #[test]
    fn file_root_is_a_construction_error() {
        let dir = tempdir().expect("tempdir");
        let file = dir.path().join("file.txt");
        write(&file, "x").expect("write");
        let err = AllowedRoots::new(vec![file]).expect_err("file root must fail");
        assert!(err.message.contains("not a directory"), "{}", err.message);
    }

    #[test]
    fn resolve_accepts_a_path_under_a_root() {
        let dir = tempdir().expect("tempdir");
        create_dir(dir.path().join("src")).expect("mkdir");
        let roots = AllowedRoots::new(vec![dir.path().to_path_buf()]).expect("roots");
        let resolved = roots
            .resolve(dir.path().join("src").to_str().expect("utf8"))
            .expect("inside root");
        assert!(resolved.ends_with("src"));
        assert!(resolved.starts_with(roots.first()));
    }

    #[test]
    fn resolve_or_first_defaults_to_the_first_root() {
        let dir = tempdir().expect("tempdir");
        let roots = AllowedRoots::new(vec![dir.path().to_path_buf()]).expect("roots");
        assert_eq!(roots.resolve_or_first(None).expect("none"), roots.first());
        assert_eq!(
            roots.resolve_or_first(Some("")).expect("empty"),
            roots.first()
        );
        assert!(roots.resolve_or_first(Some("src")).is_err());
    }

    #[test]
    fn resolve_rejects_parent_traversal_out_of_the_root() {
        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("root");
        create_dir(&root).expect("mkdir");
        write(outer.path().join("secret.txt"), "secret").expect("write");
        let roots = AllowedRoots::new(vec![root.clone()]).expect("roots");

        let traversal = root.join("..").join("secret.txt");
        let err = roots
            .resolve(traversal.to_str().expect("utf8"))
            .expect_err("traversal must be refused");
        assert!(err.contains("outside allowed directories"), "{err}");
    }

    #[test]
    fn resolve_rejects_a_relative_path() {
        let dir = tempdir().expect("tempdir");
        let roots = AllowedRoots::new(vec![dir.path().to_path_buf()]).expect("roots");
        let err = roots.resolve("src").expect_err("relative must be refused");
        assert!(err.contains("absolute"), "{err}");
    }

    #[test]
    fn resolve_gives_the_same_message_for_missing_and_outside_paths() {
        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("root");
        create_dir(&root).expect("mkdir");
        let roots = AllowedRoots::new(vec![root.clone()]).expect("roots");

        let missing = roots
            .resolve(root.join("nope").to_str().expect("utf8"))
            .expect_err("missing must be refused");
        let outside = roots
            .resolve(outer.path().to_str().expect("utf8"))
            .expect_err("outside must be refused");
        assert!(missing.contains("outside allowed directories"), "{missing}");
        assert!(outside.contains("outside allowed directories"), "{outside}");
    }

    #[cfg(unix)]
    #[test]
    fn resolve_rejects_a_symlink_escaping_the_root() {
        use std::os::unix::fs::symlink;

        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("root");
        create_dir(&root).expect("mkdir");
        let secret = outer.path().join("secret");
        create_dir(&secret).expect("mkdir");
        symlink(&secret, root.join("escape")).expect("symlink");
        let roots = AllowedRoots::new(vec![root.clone()]).expect("roots");

        let err = roots
            .resolve(root.join("escape").to_str().expect("utf8"))
            .expect_err("escaping symlink must be refused");
        assert!(err.contains("outside allowed directories"), "{err}");
    }

    #[test]
    fn accepts_a_path_under_any_of_several_roots() {
        let a = tempdir().expect("tempdir");
        let b = tempdir().expect("tempdir");
        let roots =
            AllowedRoots::new(vec![a.path().to_path_buf(), b.path().to_path_buf()]).expect("roots");
        assert!(roots.resolve(b.path().to_str().expect("utf8")).is_ok());
        assert!(roots.describe().contains(&*roots.first().to_string_lossy()));
    }

    #[test]
    fn contains_rejects_a_sibling_sharing_the_root_prefix() {
        let outer = tempdir().expect("tempdir");
        let root = outer.path().join("app");
        let sibling = outer.path().join("app-secrets");
        create_dir(&root).expect("mkdir");
        create_dir(&sibling).expect("mkdir");
        let roots = AllowedRoots::new(vec![root]).expect("roots");
        let sibling = sibling.canonicalize().expect("canonical");
        assert!(!roots.contains(&sibling));
    }
}
