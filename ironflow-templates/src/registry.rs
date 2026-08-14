//! Template discovery from a local directory.
//!
//! Scans a directory for subdirectories containing a `template.toml` file
//! and parses each manifest.
//!
//! # Examples
//!
//! ```no_run
//! use std::path::Path;
//! use ironflow_templates::registry::discover_templates;
//!
//! # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
//! let templates = discover_templates(Path::new("./my-templates"))?;
//! for (name, (manifest, _dir)) in &templates {
//!     println!("{name}: {}", manifest.template.description);
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::TemplateError;
use crate::manifest::{MANIFEST_FILENAME, TemplateManifest};

/// A discovered template: its parsed manifest and the directory it lives in.
pub type DiscoveredTemplate = (TemplateManifest, PathBuf);

/// Discover all templates in a directory.
///
/// Scans `root` for immediate subdirectories that contain a `template.toml`
/// file. Returns a map from template name to the parsed manifest and the
/// directory path, sorted alphabetically.
///
/// # Errors
///
/// Returns [`TemplateError::Io`] if the directory cannot be read, or
/// [`TemplateError::InvalidManifest`] if any `template.toml` is malformed.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use ironflow_templates::registry::discover_templates;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let templates = discover_templates(Path::new("/tmp/my-templates"))?;
/// assert!(templates.contains_key("ci-pipeline"));
/// # Ok(())
/// # }
/// ```
pub fn discover_templates(
    root: &Path,
) -> Result<BTreeMap<String, DiscoveredTemplate>, TemplateError> {
    let mut templates: BTreeMap<String, DiscoveredTemplate> = BTreeMap::new();

    let entries = fs::read_dir(root).map_err(TemplateError::Io)?;
    for entry in entries {
        let entry = entry.map_err(TemplateError::Io)?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join(MANIFEST_FILENAME);
        if !manifest_path.exists() {
            continue;
        }

        if !path.join("src").is_dir() {
            continue;
        }

        let manifest = TemplateManifest::from_file(&manifest_path)?;
        let name = manifest.template.name.clone();

        if let Some((_, existing_dir)) = templates.get(&name) {
            return Err(TemplateError::DuplicateName {
                name,
                first: existing_dir.display().to_string(),
                second: path.display().to_string(),
            });
        }

        templates.insert(name, (manifest, path));
    }

    Ok(templates)
}

/// Look up a single template by name in a directory.
///
/// # Errors
///
/// Returns [`TemplateError::TemplateNotFound`] if no template with the
/// given name exists, or [`TemplateError::NoTemplatesFound`] if the
/// directory contains no templates at all.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use ironflow_templates::registry::find_template;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let (manifest, dir) = find_template(Path::new("./templates"), "ci-pipeline")?;
/// println!("found: {}", manifest.template.description);
/// # Ok(())
/// # }
/// ```
pub fn find_template(
    root: &Path,
    name: &str,
) -> Result<(TemplateManifest, PathBuf), TemplateError> {
    let mut templates = discover_templates(root)?;

    if templates.is_empty() {
        return Err(TemplateError::NoTemplatesFound(root.display().to_string()));
    }

    match templates.remove(name) {
        Some((manifest, dir)) => Ok((manifest, dir)),
        None => {
            let available = templates.into_keys().collect::<Vec<_>>().join(", ");
            Err(TemplateError::TemplateNotFound {
                name: name.to_string(),
                available,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn create_template_dir(root: &Path, dir_name: &str, name: &str, desc: &str) {
        let dir = root.join(dir_name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(MANIFEST_FILENAME),
            format!(
                r#"[template]
name = "{name}"
description = "{desc}"
version = "1.0.0"
"#
            ),
        )
        .unwrap();
        let src = dir.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lib.rs"), "// template source").unwrap();
    }

    #[test]
    fn discover_templates_finds_all() {
        let tmp = TempDir::new().unwrap();
        create_template_dir(tmp.path(), "ci", "ci-pipeline", "CI pipeline");
        create_template_dir(tmp.path(), "review", "code-review", "Code review");

        let templates = discover_templates(tmp.path()).unwrap();
        assert_eq!(templates.len(), 2);
        assert!(templates.contains_key("ci-pipeline"));
        assert!(templates.contains_key("code-review"));
    }

    #[test]
    fn discover_templates_includes_paths() {
        let tmp = TempDir::new().unwrap();
        create_template_dir(tmp.path(), "ci", "ci-pipeline", "CI");

        let templates = discover_templates(tmp.path()).unwrap();
        let (_manifest, dir) = &templates["ci-pipeline"];
        assert!(dir.ends_with("ci"));
    }

    #[test]
    fn discover_empty_directory() {
        let tmp = TempDir::new().unwrap();
        let templates = discover_templates(tmp.path()).unwrap();
        assert!(templates.is_empty());
    }

    #[test]
    fn discover_ignores_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("README.md"), "# Templates").unwrap();
        create_template_dir(tmp.path(), "ci", "ci-pipeline", "CI");

        let templates = discover_templates(tmp.path()).unwrap();
        assert_eq!(templates.len(), 1);
    }

    #[test]
    fn discover_ignores_dirs_without_manifest() {
        let tmp = TempDir::new().unwrap();
        fs::create_dir_all(tmp.path().join("empty-dir")).unwrap();
        create_template_dir(tmp.path(), "ci", "ci-pipeline", "CI");

        let templates = discover_templates(tmp.path()).unwrap();
        assert_eq!(templates.len(), 1);
    }

    #[test]
    fn find_template_returns_manifest_and_dir() {
        let tmp = TempDir::new().unwrap();
        create_template_dir(tmp.path(), "ci", "ci-pipeline", "CI");

        let (manifest, dir) = find_template(tmp.path(), "ci-pipeline").unwrap();
        assert_eq!(manifest.template.name, "ci-pipeline");
        assert!(dir.ends_with("ci"));
    }

    #[test]
    fn find_template_not_found() {
        let tmp = TempDir::new().unwrap();
        create_template_dir(tmp.path(), "ci", "ci-pipeline", "CI");

        let err = find_template(tmp.path(), "nope").unwrap_err();
        assert!(err.to_string().contains("nope"));
        assert!(err.to_string().contains("ci-pipeline"));
    }

    #[test]
    fn find_template_empty_repo() {
        let tmp = TempDir::new().unwrap();
        let err = find_template(tmp.path(), "anything").unwrap_err();
        assert!(err.to_string().contains("no templates found"));
    }

    #[test]
    fn discover_rejects_duplicate_names() {
        let tmp = TempDir::new().unwrap();
        create_template_dir(tmp.path(), "dir-a", "same-name", "First");
        create_template_dir(tmp.path(), "dir-b", "same-name", "Second");

        let err = discover_templates(tmp.path()).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("same-name"),
            "error should name the duplicate: {msg}"
        );
        assert!(
            msg.contains("dir-a") || msg.contains("dir-b"),
            "error should name a directory: {msg}"
        );
    }

    #[test]
    fn discover_ignores_dirs_without_src() {
        let tmp = TempDir::new().unwrap();

        let dir = tmp.path().join("no-src");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join(MANIFEST_FILENAME),
            r#"[template]
name = "no-src"
description = "Missing src dir"
version = "1.0.0"
"#,
        )
        .unwrap();

        create_template_dir(tmp.path(), "valid", "valid-template", "Valid");

        let templates = discover_templates(tmp.path()).unwrap();
        assert_eq!(templates.len(), 1);
        assert!(templates.contains_key("valid-template"));
    }
}
