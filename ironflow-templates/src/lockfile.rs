//! Lockfile tracking for installed templates (`.ironflow-templates.lock`).
//!
//! Records which templates have been installed, their versions, and where
//! they live on disk. Used by `template update` to compare against the
//! registry and propose upgrades.
//!
//! # Format
//!
//! ```toml
//! [[installed]]
//! name = "hello-world"
//! version = "1.2.0"
//! repo = "https://gitlab.com/ironflow/templates"
//! installed_at = "2026-09-10"
//! path = "src/workflows/hello_world.rs"
//! ```
//!
//! # Examples
//!
//! ```no_run
//! use std::path::Path;
//! use ironflow_templates::lockfile::LockFile;
//!
//! # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
//! let mut lock = LockFile::load(Path::new(".ironflow-templates.lock"))?;
//! for entry in lock.installed() {
//!     println!("{}: v{}", entry.name, entry.version);
//! }
//! # Ok(())
//! # }
//! ```

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::TemplateError;

/// Filename for the lockfile at the project root.
pub const LOCKFILE_NAME: &str = ".ironflow-templates.lock";

/// A lockfile tracking installed templates.
///
/// # Examples
///
/// ```
/// use ironflow_templates::lockfile::LockFile;
///
/// let mut lock = LockFile::default();
/// assert!(lock.installed().is_empty());
/// ```
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LockFile {
    /// Installed template entries.
    #[serde(default)]
    installed: Vec<InstalledEntry>,
}

/// A single installed template entry.
///
/// # Examples
///
/// ```
/// use ironflow_templates::lockfile::InstalledEntry;
///
/// let entry = InstalledEntry {
///     name: "hello-world".to_string(),
///     version: "1.0.0".to_string(),
///     repo: "https://gitlab.com/ironflow/templates".to_string(),
///     installed_at: "2026-09-10".to_string(),
///     path: "src/workflows/hello_world".to_string(),
/// };
/// assert_eq!(entry.name, "hello-world");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstalledEntry {
    /// Template name.
    pub name: String,
    /// Installed version.
    pub version: String,
    /// Source repository URL.
    pub repo: String,
    /// Date the template was installed (ISO 8601 date).
    pub installed_at: String,
    /// Relative path where the template was installed.
    pub path: String,
}

impl LockFile {
    /// Load a lockfile from disk, or return an empty lockfile if the file
    /// does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::Lockfile`] if the file exists but cannot
    /// be parsed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// use ironflow_templates::lockfile::LockFile;
    ///
    /// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
    /// let lock = LockFile::load(Path::new(".ironflow-templates.lock"))?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn load(path: &Path) -> Result<Self, TemplateError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(path).map_err(TemplateError::Io)?;
        toml::from_str(&content)
            .map_err(|e| TemplateError::Lockfile(format!("failed to parse lockfile: {e}")))
    }

    /// Save the lockfile to disk.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::Lockfile`] if serialization fails, or
    /// [`TemplateError::Io`] on write failure.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use std::path::Path;
    /// use ironflow_templates::lockfile::LockFile;
    ///
    /// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
    /// let lock = LockFile::default();
    /// lock.save(Path::new(".ironflow-templates.lock"))?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn save(&self, path: &Path) -> Result<(), TemplateError> {
        let content = toml::to_string_pretty(self)
            .map_err(|e| TemplateError::Lockfile(format!("failed to serialize lockfile: {e}")))?;
        fs::write(path, content).map_err(TemplateError::Io)
    }

    /// Record a new template installation.
    ///
    /// If a template with the same name already exists, it is replaced.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_templates::lockfile::{LockFile, InstalledEntry};
    ///
    /// let mut lock = LockFile::default();
    /// lock.record_install(InstalledEntry {
    ///     name: "ci-pipeline".to_string(),
    ///     version: "1.0.0".to_string(),
    ///     repo: "https://example.com/templates".to_string(),
    ///     installed_at: "2026-09-10".to_string(),
    ///     path: "src/workflows/ci_pipeline".to_string(),
    /// });
    /// assert_eq!(lock.installed().len(), 1);
    /// ```
    pub fn record_install(&mut self, entry: InstalledEntry) {
        self.installed.retain(|e| e.name != entry.name);
        self.installed.push(entry);
    }

    /// Look up an installed template by name.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_templates::lockfile::{LockFile, InstalledEntry};
    ///
    /// let mut lock = LockFile::default();
    /// lock.record_install(InstalledEntry {
    ///     name: "hello".to_string(),
    ///     version: "1.0.0".to_string(),
    ///     repo: "https://example.com".to_string(),
    ///     installed_at: "2026-09-10".to_string(),
    ///     path: "src/workflows/hello".to_string(),
    /// });
    /// assert!(lock.find_installed("hello").is_some());
    /// assert!(lock.find_installed("nope").is_none());
    /// ```
    pub fn find_installed(&self, name: &str) -> Option<&InstalledEntry> {
        self.installed.iter().find(|e| e.name == name)
    }

    /// List all installed templates.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironflow_templates::lockfile::LockFile;
    ///
    /// let lock = LockFile::default();
    /// assert!(lock.installed().is_empty());
    /// ```
    pub fn installed(&self) -> &[InstalledEntry] {
        &self.installed
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn sample_entry(name: &str, version: &str) -> InstalledEntry {
        InstalledEntry {
            name: name.to_string(),
            version: version.to_string(),
            repo: "https://example.com/templates".to_string(),
            installed_at: "2026-09-10".to_string(),
            path: format!("src/workflows/{name}"),
        }
    }

    #[test]
    fn round_trip() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(LOCKFILE_NAME);

        let mut lock = LockFile::default();
        lock.record_install(sample_entry("hello", "1.0.0"));
        lock.record_install(sample_entry("deploy", "2.0.0"));
        lock.save(&path).unwrap();

        let loaded = LockFile::load(&path).unwrap();
        assert_eq!(loaded.installed().len(), 2);
        assert_eq!(loaded.find_installed("hello").unwrap().version, "1.0.0");
        assert_eq!(loaded.find_installed("deploy").unwrap().version, "2.0.0");
    }

    #[test]
    fn record_install_replaces_existing() {
        let mut lock = LockFile::default();
        lock.record_install(sample_entry("hello", "1.0.0"));
        lock.record_install(sample_entry("hello", "2.0.0"));

        assert_eq!(lock.installed().len(), 1);
        assert_eq!(lock.find_installed("hello").unwrap().version, "2.0.0");
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let tmp = TempDir::new().unwrap();
        let lock = LockFile::load(&tmp.path().join("nope.lock")).unwrap();
        assert!(lock.installed().is_empty());
    }

    #[test]
    fn load_invalid_toml_returns_error() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(LOCKFILE_NAME);
        fs::write(&path, "not valid toml [[[").unwrap();

        let err = LockFile::load(&path).unwrap_err();
        assert!(err.to_string().contains("lockfile"));
    }

    #[test]
    fn find_installed_returns_none_for_unknown() {
        let lock = LockFile::default();
        assert!(lock.find_installed("unknown").is_none());
    }

    #[test]
    fn record_install_preserves_other_entries() {
        let mut lock = LockFile::default();
        lock.record_install(sample_entry("a", "1.0.0"));
        lock.record_install(sample_entry("b", "1.0.0"));
        lock.record_install(sample_entry("a", "2.0.0"));

        assert_eq!(lock.installed().len(), 2);
        assert_eq!(lock.find_installed("a").unwrap().version, "2.0.0");
        assert_eq!(lock.find_installed("b").unwrap().version, "1.0.0");
    }
}
