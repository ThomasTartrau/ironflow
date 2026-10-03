//! Install a template into the user's project.
//!
//! Copies the template source files from a discovered template directory
//! into the user's project, following the shadcn pattern.
//!
//! # Examples
//!
//! ```no_run
//! use std::path::Path;
//! use ironflow_templates::manifest::TemplateManifest;
//! use ironflow_templates::install::install_template;
//!
//! # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
//! let manifest = TemplateManifest::from_file(Path::new("/tmp/repo/ci-pipeline/template.toml"))?;
//! install_template(
//!     &manifest,
//!     Path::new("/tmp/repo/ci-pipeline"),
//!     Path::new("./src/workflows/ci-pipeline"),
//! )?;
//! # Ok(())
//! # }
//! ```

use std::fs;
use std::io::{Error, ErrorKind};
use std::path::{Path, PathBuf};

use tempfile::Builder;

use crate::error::TemplateError;
use crate::manifest::{DependencySpec, TemplateManifest};

/// Result of a successful template installation.
#[derive(Debug)]
pub struct InstallResult {
    /// Where the template files were copied to.
    pub destination: PathBuf,
    /// Dependencies the user should add to their `Cargo.toml`.
    pub dependencies: Vec<String>,
}

/// Install a template by copying its `src/` directory to `destination`.
///
/// The manifest is passed in to avoid re-reading `template.toml` from disk
/// when the caller already parsed it during discovery.
///
/// # Errors
///
/// Returns [`TemplateError::AlreadyInstalled`] if `destination` already
/// exists, or [`TemplateError::Io`] on filesystem errors.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use ironflow_templates::manifest::TemplateManifest;
/// use ironflow_templates::install::install_template;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let manifest = TemplateManifest::from_file(Path::new("/tmp/repo/ci-pipeline/template.toml"))?;
/// install_template(
///     &manifest,
///     Path::new("/tmp/repo/ci-pipeline"),
///     Path::new("./src/workflows/ci-pipeline"),
/// )?;
/// # Ok(())
/// # }
/// ```
pub fn install_template(
    manifest: &TemplateManifest,
    template_dir: &Path,
    destination: &Path,
) -> Result<InstallResult, TemplateError> {
    if destination.exists() {
        return Err(TemplateError::AlreadyInstalled(
            destination.display().to_string(),
        ));
    }

    let src_dir = template_dir.join("src");
    if !src_dir.exists() {
        return Err(TemplateError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("template source directory not found: {}", src_dir.display()),
        )));
    }

    copy_dir_recursive(&src_dir, destination)?;

    let dependencies = format_dependencies(manifest);

    Ok(InstallResult {
        destination: destination.to_path_buf(),
        dependencies,
    })
}

/// Result of a successful in-place template replacement.
#[derive(Debug)]
pub struct ReplaceResult {
    /// Directory whose content was replaced.
    pub destination: PathBuf,
    /// Files present in the new version only, relative to `destination`, sorted.
    pub added: Vec<String>,
    /// Files present in both versions with different bytes, sorted.
    pub modified: Vec<String>,
    /// Files present in the old copy only (removed upstream or local), sorted.
    pub removed: Vec<String>,
    /// Dependencies the user should add to their `Cargo.toml`.
    pub dependencies: Vec<String>,
}

/// Replace an installed template with the `src/` directory of a new version.
///
/// The new files are staged next to `destination` (same filesystem), then
/// swapped in with two renames. If the swap fails, the old directory is put
/// back, so `destination` is never left half-written.
///
/// # Errors
///
/// Returns [`TemplateError::Io`] if `destination` does not exist, if the
/// template has no `src/` directory, or on filesystem errors. In every error
/// case the old content of `destination` is preserved.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use ironflow_templates::manifest::TemplateManifest;
/// use ironflow_templates::install::replace_template;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let manifest = TemplateManifest::from_file(Path::new("/tmp/repo/ci-pipeline/template.toml"))?;
/// let result = replace_template(
///     &manifest,
///     Path::new("/tmp/repo/ci-pipeline"),
///     Path::new("./src/workflows/ci_pipeline"),
/// )?;
/// println!("{} file(s) added", result.added.len());
/// # Ok(())
/// # }
/// ```
pub fn replace_template(
    manifest: &TemplateManifest,
    template_dir: &Path,
    destination: &Path,
) -> Result<ReplaceResult, TemplateError> {
    if !destination.is_dir() {
        return Err(TemplateError::Io(Error::new(
            ErrorKind::NotFound,
            format!("installed directory not found: {}", destination.display()),
        )));
    }

    let src_dir = template_dir.join("src");
    if !src_dir.exists() {
        return Err(TemplateError::Io(Error::new(
            ErrorKind::NotFound,
            format!("template source directory not found: {}", src_dir.display()),
        )));
    }

    let parent = destination
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));

    let staging = Builder::new()
        .prefix(".replace-staging-")
        .tempdir_in(parent)
        .map_err(TemplateError::Io)?;
    copy_dir_recursive(&src_dir, staging.path())?;

    let old_files = list_files(destination)?;
    let new_files = list_files(staging.path())?;
    let added = new_files
        .iter()
        .filter(|f| !old_files.contains(f))
        .cloned()
        .collect();
    let removed = old_files
        .iter()
        .filter(|f| !new_files.contains(f))
        .cloned()
        .collect();
    let mut modified = Vec::new();
    for file in new_files.iter().filter(|f| old_files.contains(f)) {
        let old = fs::read(destination.join(file)).map_err(TemplateError::Io)?;
        let new = fs::read(staging.path().join(file)).map_err(TemplateError::Io)?;
        if old != new {
            modified.push(file.clone());
        }
    }

    let backup = Builder::new()
        .prefix(".replace-backup-")
        .tempdir_in(parent)
        .map_err(TemplateError::Io)?;
    let backup_path = backup.path().join("old");
    fs::rename(destination, &backup_path).map_err(TemplateError::Io)?;

    // `keep` hands the directory over: it must survive as the new destination.
    let staged_path = staging.keep();
    if let Err(e) = fs::rename(&staged_path, destination) {
        // The swap error is the one worth reporting; restoring is best effort.
        return Err(match fs::rename(&backup_path, destination) {
            Ok(()) => {
                drop(fs::remove_dir_all(&staged_path));
                TemplateError::Io(e)
            }
            Err(restore) => {
                // Restoring failed: the backup holds the only copy of the old
                // template, so it must outlive this function.
                let kept = backup.keep().join("old");
                TemplateError::Io(Error::other(format!(
                    "failed to install update ({e}) and to restore the previous version ({restore}); previous files kept in {}",
                    kept.display()
                )))
            }
        });
    }
    fs::remove_dir_all(backup.path()).map_err(TemplateError::Io)?;

    Ok(ReplaceResult {
        destination: destination.to_path_buf(),
        added,
        modified,
        removed,
        dependencies: format_dependencies(manifest),
    })
}

/// Relative paths (with `/` separators) of every file under `root`, sorted.
fn list_files(root: &Path) -> Result<Vec<String>, TemplateError> {
    let mut files = Vec::new();
    collect_files(root, "", &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(dir: &Path, prefix: &str, out: &mut Vec<String>) -> Result<(), TemplateError> {
    for entry in fs::read_dir(dir).map_err(TemplateError::Io)? {
        let entry = entry.map_err(TemplateError::Io)?;
        let path = entry.path();
        let rel = format!("{prefix}{}", entry.file_name().to_string_lossy());
        if path.is_dir() {
            collect_files(&path, &format!("{rel}/"), out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

/// Where `template add` installs a template when no output is given.
///
/// The directory is named after the Rust module the handler is registered
/// under (`gitlab-mr-review` becomes `gitlab_mr_review`), since `mod` must
/// find it. When `project_root/src/lib.rs` or `src/handlers.rs` defines
/// `handlers()` (the layout `ironflow` scaffolds), the module goes next to it
/// in `src/`; otherwise under `src/workflows/`.
///
/// # Examples
///
/// ```
/// use std::path::Path;
/// use ironflow_templates::install::default_destination;
///
/// let dest = default_destination(Path::new("/nonexistent"), "gitlab-mr-review");
/// assert_eq!(dest, Path::new("/nonexistent/src/workflows/gitlab_mr_review"));
/// ```
pub fn default_destination(project_root: &Path, name: &str) -> PathBuf {
    let module = name.replace('-', "_");
    let src = project_root.join("src");
    let handlers_in_src = ["lib.rs", "handlers.rs"].iter().any(|file| {
        fs::read_to_string(src.join(file)).is_ok_and(|content| content.contains("fn handlers()"))
    });

    if handlers_in_src {
        src.join(module)
    } else {
        src.join("workflows").join(module)
    }
}

/// Recursively copy a directory and its contents.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), TemplateError> {
    fs::create_dir_all(dst).map_err(TemplateError::Io)?;

    for entry in fs::read_dir(src).map_err(TemplateError::Io)? {
        let entry = entry.map_err(TemplateError::Io)?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(TemplateError::Io)?;
        }
    }

    Ok(())
}

/// Format dependency lines for display to the user.
fn format_dependencies(manifest: &TemplateManifest) -> Vec<String> {
    let mut deps: Vec<_> = manifest
        .dependencies
        .iter()
        .map(|(name, spec)| match spec {
            DependencySpec::Simple(version) => {
                format!("{name} = \"{version}\"")
            }
            DependencySpec::Detailed(d) if d.features.is_empty() => {
                format!("{name} = \"{}\"", d.version)
            }
            DependencySpec::Detailed(d) => {
                let features = d
                    .features
                    .iter()
                    .map(|f| format!("\"{f}\""))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{name} = {{ version = \"{}\", features = [{features}] }}",
                    d.version
                )
            }
        })
        .collect();
    deps.sort();
    deps
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::manifest::MANIFEST_FILENAME;

    fn setup_template(tmp: &TempDir) -> (TemplateManifest, std::path::PathBuf) {
        let template_dir = tmp.path().join("ci-pipeline");
        let src = template_dir.join("src");
        fs::create_dir_all(&src).unwrap();
        let manifest_content = r#"[template]
name = "ci-pipeline"
description = "CI pipeline"
version = "1.0.0"

[dependencies]
serde = { version = "1", features = ["derive"] }
tokio = "1"
"#;
        fs::write(template_dir.join(MANIFEST_FILENAME), manifest_content).unwrap();
        fs::write(src.join("ci_pipeline.rs"), "// CI pipeline handler").unwrap();
        fs::write(src.join("helpers.rs"), "// helpers").unwrap();
        let manifest = TemplateManifest::parse(manifest_content).unwrap();
        (manifest, template_dir)
    }

    #[test]
    fn install_copies_files() {
        let tmp = TempDir::new().unwrap();
        let (manifest, template_dir) = setup_template(&tmp);
        let dest = tmp.path().join("output");

        let result = install_template(&manifest, &template_dir, &dest).unwrap();
        assert!(dest.join("ci_pipeline.rs").exists());
        assert!(dest.join("helpers.rs").exists());
        assert_eq!(result.dependencies.len(), 2);
        assert!(result.dependencies.iter().any(|d| d.contains("serde")));
        assert!(result.dependencies.iter().any(|d| d.contains("tokio")));
    }

    #[test]
    fn install_refuses_existing_destination() {
        let tmp = TempDir::new().unwrap();
        let (manifest, template_dir) = setup_template(&tmp);
        let dest = tmp.path().join("output");
        fs::create_dir_all(&dest).unwrap();

        let err = install_template(&manifest, &template_dir, &dest).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn install_copies_nested_directories() {
        let tmp = TempDir::new().unwrap();
        let (manifest, template_dir) = setup_template(&tmp);
        let nested = template_dir.join("src").join("utils");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("helper.rs"), "// nested helper").unwrap();

        let dest = tmp.path().join("output");
        install_template(&manifest, &template_dir, &dest).unwrap();

        assert!(dest.join("utils").join("helper.rs").exists());
    }

    #[test]
    fn default_destination_is_a_rust_module_name() {
        let tmp = TempDir::new().unwrap();

        let dest = default_destination(tmp.path(), "gitlab-mr-review");

        assert_eq!(
            dest,
            tmp.path()
                .join("src")
                .join("workflows")
                .join("gitlab_mr_review")
        );
    }

    #[test]
    fn default_destination_sits_next_to_handlers_in_src() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(
            src.join("lib.rs"),
            "pub fn handlers() -> Vec<Box<dyn WorkflowHandler>> {\n    vec![]\n}\n",
        )
        .unwrap();

        let dest = default_destination(tmp.path(), "gitlab-mr-review");

        assert_eq!(dest, src.join("gitlab_mr_review"));
    }

    #[test]
    fn default_destination_ignores_lib_without_handlers() {
        let tmp = TempDir::new().unwrap();
        let src = tmp.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("lib.rs"), "pub mod other;\n").unwrap();

        let dest = default_destination(tmp.path(), "ci");

        assert_eq!(dest, src.join("workflows").join("ci"));
    }

    #[test]
    fn replace_reports_added_modified_and_removed() {
        let tmp = TempDir::new().unwrap();
        let (manifest, template_dir) = setup_template(&tmp);
        let dest = tmp.path().join("output");
        fs::create_dir_all(dest.join("old_dir")).unwrap();
        fs::write(dest.join("ci_pipeline.rs"), "// old handler").unwrap();
        fs::write(dest.join("helpers.rs"), "// helpers").unwrap();
        fs::write(dest.join("old_dir").join("gone.rs"), "// gone").unwrap();
        fs::write(template_dir.join("src").join("new.rs"), "// new").unwrap();

        let result = replace_template(&manifest, &template_dir, &dest).unwrap();

        assert_eq!(result.added, vec!["new.rs".to_string()]);
        assert_eq!(result.modified, vec!["ci_pipeline.rs".to_string()]);
        assert_eq!(result.removed, vec!["old_dir/gone.rs".to_string()]);
        assert_eq!(result.dependencies.len(), 2);
        assert_eq!(
            fs::read_to_string(dest.join("ci_pipeline.rs")).unwrap(),
            "// CI pipeline handler"
        );
        assert!(!dest.join("old_dir").exists());
        let leftovers = fs::read_dir(tmp.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".replace-")
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn replace_requires_existing_destination() {
        let tmp = TempDir::new().unwrap();
        let (manifest, template_dir) = setup_template(&tmp);
        let dest = tmp.path().join("missing");

        let err = replace_template(&manifest, &template_dir, &dest).unwrap_err();

        assert!(err.to_string().contains("not found"));
        assert!(!dest.exists());
    }

    #[test]
    fn replace_leaves_destination_untouched_without_src() {
        let tmp = TempDir::new().unwrap();
        let (manifest, template_dir) = setup_template(&tmp);
        fs::remove_dir_all(template_dir.join("src")).unwrap();
        let dest = tmp.path().join("output");
        fs::create_dir_all(&dest).unwrap();
        fs::write(dest.join("keep.rs"), "// keep").unwrap();

        let err = replace_template(&manifest, &template_dir, &dest).unwrap_err();

        assert!(err.to_string().contains("source directory not found"));
        assert_eq!(fs::read_to_string(dest.join("keep.rs")).unwrap(), "// keep");
    }

    #[test]
    fn format_dependencies_sorts_alphabetically() {
        let toml = r#"
[template]
name = "test"
description = "test"
version = "1.0.0"

[dependencies]
z-crate = "1"
a-crate = "2"
"#;
        let manifest = TemplateManifest::parse(toml).unwrap();
        let deps = format_dependencies(&manifest);
        assert_eq!(deps[0], "a-crate = \"2\"");
        assert_eq!(deps[1], "z-crate = \"1\"");
    }
}
