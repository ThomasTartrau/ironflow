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
use std::path::{Path, PathBuf};

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
