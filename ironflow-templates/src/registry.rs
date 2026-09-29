//! Template discovery from a local directory and remote registry.
//!
//! Scans a directory for subdirectories containing a `template.toml` file
//! and parses each manifest. Also supports fetching a remote registry
//! index (`index.toml`) to resolve template names without a full URL.
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

use std::env;

use semver::Version;
use serde::{Deserialize, Serialize};

use crate::error::{NameList, TemplateError};
use crate::fetch::{fetch_repo, validate_git_url};
use crate::manifest::{MANIFEST_FILENAME, TemplateManifest};

/// Default template registry URL.
pub const DEFAULT_REGISTRY_URL: &str = "https://gitlab.com/ironflow/registry";

/// Resolve the registry URL from an explicit value, the `IRONFLOW_REGISTRY_URL`
/// environment variable, or the built-in default.
///
/// # Examples
///
/// ```
/// use ironflow_templates::registry::resolve_registry_url;
///
/// let url = resolve_registry_url(None);
/// assert!(url.starts_with("https://"));
/// ```
pub fn resolve_registry_url(explicit: Option<&str>) -> String {
    explicit
        .map(String::from)
        .or_else(|| env::var("IRONFLOW_REGISTRY_URL").ok())
        .unwrap_or_else(|| DEFAULT_REGISTRY_URL.to_string())
}

/// A discovered template: its parsed manifest and the directory it lives in.
pub type DiscoveredTemplate = (TemplateManifest, PathBuf);

/// Discover all templates in a directory.
///
/// Looks for a `template.toml` next to a `src/` directory in `root` itself
/// (a repository holding a single template) and in each of its immediate
/// subdirectories (a repository holding several). Returns a map from template
/// name to the parsed manifest and the directory path, sorted alphabetically.
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

    insert_template_dir(&mut templates, root.to_path_buf())?;

    let entries = fs::read_dir(root).map_err(TemplateError::Io)?;
    for entry in entries {
        let entry = entry.map_err(TemplateError::Io)?;
        let path = entry.path();

        if path.is_dir() {
            insert_template_dir(&mut templates, path)?;
        }
    }

    Ok(templates)
}

/// Add `dir` to `templates` when it holds a `template.toml` and a `src/`.
fn insert_template_dir(
    templates: &mut BTreeMap<String, DiscoveredTemplate>,
    dir: PathBuf,
) -> Result<(), TemplateError> {
    let manifest_path = dir.join(MANIFEST_FILENAME);
    if !manifest_path.exists() || !dir.join("src").is_dir() {
        return Ok(());
    }

    let manifest = TemplateManifest::from_file(&manifest_path)?;
    let name = manifest.template.name.clone();

    if let Some((_, existing_dir)) = templates.get(&name) {
        return Err(TemplateError::DuplicateName {
            name,
            first: existing_dir.display().to_string(),
            second: dir.display().to_string(),
        });
    }

    templates.insert(name, (manifest, dir));
    Ok(())
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
            let available = NameList(templates.into_keys().collect());
            Err(TemplateError::TemplateNotFound {
                name: name.to_string(),
                available,
            })
        }
    }
}

/// The filename expected inside a registry repository.
pub const REGISTRY_INDEX_FILENAME: &str = "index.toml";

/// A remote template registry index parsed from `index.toml`.
///
/// # Examples
///
/// ```
/// use ironflow_templates::registry::RegistryIndex;
///
/// let toml = r#"
/// [[templates]]
/// name = "hello-world"
/// description = "Minimal workflow handler"
/// category = "getting-started"
/// repo = "https://gitlab.com/ironflow/templates"
/// "#;
///
/// let index: RegistryIndex = toml::from_str(toml)?;
/// assert_eq!(index.templates.len(), 1);
/// assert_eq!(index.templates[0].name, "hello-world");
/// # Ok::<(), toml::de::Error>(())
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryIndex {
    /// Available templates in the registry.
    #[serde(default)]
    pub templates: Vec<RegistryEntry>,
}

/// A single entry in the remote registry index.
///
/// # Examples
///
/// ```
/// use semver::Version;
/// use ironflow_templates::registry::RegistryEntry;
///
/// let entry = RegistryEntry {
///     name: "ci-pipeline".to_string(),
///     description: "Generic CI pipeline".to_string(),
///     category: Some("ci-cd".to_string()),
///     repo: "https://gitlab.com/ironflow/templates".to_string(),
///     min_ironflow_version: Some(Version::new(0, 5, 0)),
///     authors: vec!["Alice".to_string()],
///     latest_version: Some(Version::new(1, 2, 0)),
/// };
/// assert_eq!(entry.name, "ci-pipeline");
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Template name.
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Category for UI grouping.
    #[serde(default)]
    pub category: Option<String>,
    /// Git repository URL containing this template.
    pub repo: String,
    /// Minimum Ironflow version required.
    #[serde(default)]
    pub min_ironflow_version: Option<Version>,
    /// Template authors.
    #[serde(default)]
    pub authors: Vec<String>,
    /// Latest published version.
    #[serde(default)]
    pub latest_version: Option<Version>,
}

/// Fetch and parse a remote registry index from a Git repository.
///
/// Clones the repository and reads `index.toml` from its root.
///
/// # Errors
///
/// Returns [`TemplateError::RegistryNotFound`] if the repository does not
/// exist or has no `index.toml`, [`TemplateError::RegistryUnreachable`] if
/// the host cannot be reached, [`TemplateError::Git`] if the URL is rejected,
/// or [`TemplateError::Registry`] if the index is invalid.
///
/// # Examples
///
/// ```no_run
/// use ironflow_templates::registry::fetch_registry_index;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let index = fetch_registry_index("https://gitlab.com/ironflow/registry")?;
/// for entry in &index.templates {
///     println!("{}: {}", entry.name, entry.description);
/// }
/// # Ok(())
/// # }
/// ```
pub fn fetch_registry_index(url: &str) -> Result<RegistryIndex, TemplateError> {
    validate_git_url(url)?;
    let tmp = fetch_repo(url).map_err(|e| match e {
        TemplateError::Git(message) if is_missing_repository(&message) => {
            TemplateError::RegistryNotFound {
                url: url.to_string(),
                reason: message,
            }
        }
        TemplateError::Git(message) => TemplateError::RegistryUnreachable {
            url: url.to_string(),
            reason: message,
        },
        other => other,
    })?;
    let index_path = tmp.path().join(REGISTRY_INDEX_FILENAME);

    if !index_path.exists() {
        return Err(TemplateError::RegistryNotFound {
            url: url.to_string(),
            reason: format!("the repository has no {REGISTRY_INDEX_FILENAME} at its root"),
        });
    }

    let content = fs::read_to_string(&index_path).map_err(TemplateError::Io)?;
    parse_registry_index(&content)
}

/// `true` when a failed `git clone` reached the remote, which answered that
/// the repository does not exist (GitLab and GitHub say "not found", a local
/// path "does not appear to be a git repository" or "does not exist").
fn is_missing_repository(git_stderr: &str) -> bool {
    let stderr = git_stderr.to_lowercase();
    [
        "not found",
        "does not appear to be a git repository",
        "does not exist",
    ]
    .iter()
    .any(|pattern| stderr.contains(pattern))
}

/// Parse a registry index from a TOML string.
///
/// # Errors
///
/// Returns [`TemplateError::Registry`] if the TOML is malformed.
///
/// # Examples
///
/// ```
/// use ironflow_templates::registry::parse_registry_index;
///
/// let index = parse_registry_index(r#"
/// [[templates]]
/// name = "hello"
/// description = "Hello world"
/// repo = "https://example.com/templates"
/// "#)?;
/// assert_eq!(index.templates[0].name, "hello");
/// # Ok::<(), ironflow_templates::error::TemplateError>(())
/// ```
pub fn parse_registry_index(toml_str: &str) -> Result<RegistryIndex, TemplateError> {
    toml::from_str(toml_str)
        .map_err(|e| TemplateError::Registry(format!("invalid registry index: {e}")))
}

/// Resolve a template by name from a registry index.
///
/// # Errors
///
/// Returns [`TemplateError::NotInRegistry`] if no template with the given
/// name exists in the index.
///
/// # Examples
///
/// ```
/// use ironflow_templates::registry::{RegistryIndex, RegistryEntry, resolve_template_entry};
///
/// let index = RegistryIndex {
///     templates: vec![RegistryEntry {
///         name: "hello".to_string(),
///         description: "Hello world".to_string(),
///         category: None,
///         repo: "https://example.com/templates".to_string(),
///         min_ironflow_version: None,
///         authors: vec![],
///         latest_version: None,
///     }],
/// };
///
/// let entry = resolve_template_entry(&index, "hello")?;
/// assert_eq!(entry.repo, "https://example.com/templates");
/// # Ok::<(), ironflow_templates::error::TemplateError>(())
/// ```
pub fn resolve_template_entry<'a>(
    index: &'a RegistryIndex,
    name: &str,
) -> Result<&'a RegistryEntry, TemplateError> {
    index
        .templates
        .iter()
        .find(|e| e.name == name)
        .ok_or_else(|| {
            let available = NameList(index.templates.iter().map(|e| e.name.clone()).collect());
            TemplateError::NotInRegistry {
                name: name.to_string(),
                available,
            }
        })
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

    fn create_root_template(root: &Path, name: &str) {
        fs::write(
            root.join(MANIFEST_FILENAME),
            format!(
                r#"[template]
name = "{name}"
description = "Root template"
version = "0.1.0"
"#
            ),
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src").join("mod.rs"), "// template source").unwrap();
    }

    #[test]
    fn discover_finds_template_at_repository_root() {
        let tmp = TempDir::new().unwrap();
        create_root_template(tmp.path(), "gitlab-mr-review");

        let templates = discover_templates(tmp.path()).unwrap();
        assert_eq!(templates.len(), 1);
        let (manifest, dir) = &templates["gitlab-mr-review"];
        assert_eq!(manifest.template.description, "Root template");
        assert_eq!(dir, tmp.path());
    }

    #[test]
    fn discover_combines_root_and_subdirectory_templates() {
        let tmp = TempDir::new().unwrap();
        create_root_template(tmp.path(), "root-one");
        create_template_dir(tmp.path(), "ci", "ci-pipeline", "CI");

        let templates = discover_templates(tmp.path()).unwrap();
        assert_eq!(
            templates.keys().collect::<Vec<_>>(),
            vec!["ci-pipeline", "root-one"]
        );
    }

    #[test]
    fn discover_ignores_root_manifest_without_src() {
        let tmp = TempDir::new().unwrap();
        create_root_template(tmp.path(), "no-src");
        fs::remove_dir_all(tmp.path().join("src")).unwrap();

        let templates = discover_templates(tmp.path()).unwrap();
        assert!(templates.is_empty());
    }

    #[test]
    fn discover_rejects_root_and_subdirectory_with_same_name() {
        let tmp = TempDir::new().unwrap();
        create_root_template(tmp.path(), "same-name");
        create_template_dir(tmp.path(), "nested", "same-name", "Nested");

        let err = discover_templates(tmp.path()).unwrap_err();
        assert!(err.to_string().contains("same-name"), "got: {err}");
    }

    #[test]
    fn fetch_registry_missing_repository_is_not_found() {
        let tmp = TempDir::new().unwrap();
        let url = format!("file://{}", tmp.path().join("absent").display());

        let err = fetch_registry_index(&url).unwrap_err();
        assert!(
            matches!(err, TemplateError::RegistryNotFound { .. }),
            "got: {err:?}"
        );
        assert!(err.to_string().contains(&url), "got: {err}");
    }

    #[test]
    fn fetch_registry_without_index_is_not_found() {
        let tmp = TempDir::new().unwrap();
        let repo = tmp.path().join("registry");
        fs::create_dir_all(&repo).unwrap();
        fs::write(repo.join("README.md"), "no index here").unwrap();
        for args in [
            vec!["init", "-q"],
            vec!["add", "."],
            vec![
                "-c",
                "user.name=t",
                "-c",
                "user.email=t@t",
                "commit",
                "-q",
                "-m",
                "init",
            ],
        ] {
            let status = std::process::Command::new("git")
                .args(&args)
                .current_dir(&repo)
                .status()
                .unwrap();
            assert!(status.success());
        }
        let url = format!("file://{}", repo.display());

        let err = fetch_registry_index(&url).unwrap_err();
        assert!(
            matches!(err, TemplateError::RegistryNotFound { .. }),
            "got: {err:?}"
        );
        assert!(
            err.to_string().contains(REGISTRY_INDEX_FILENAME),
            "got: {err}"
        );
    }

    #[test]
    fn fetch_registry_on_a_closed_port_is_unreachable() {
        // Port 1 is never listening on the loopback interface.
        let url = "http://127.0.0.1:1/ironflow/registry";

        let err = fetch_registry_index(url).unwrap_err();
        assert!(
            matches!(err, TemplateError::RegistryUnreachable { .. }),
            "got: {err:?}"
        );
    }

    #[test]
    fn gitlab_project_not_found_message_is_classified_as_not_found() {
        let stderr = "remote: The project you were looking for could not be found or you \
                      don't have permission to view it.\nfatal: repository \
                      'https://gitlab.com/ironflow/registry.git/' not found";
        assert!(is_missing_repository(stderr));
        assert!(!is_missing_repository(
            "fatal: unable to access 'https://gitlab.com/x/': Could not resolve host: gitlab.com"
        ));
    }

    #[test]
    fn default_registry_is_the_ironflow_group_registry() {
        assert_eq!(DEFAULT_REGISTRY_URL, "https://gitlab.com/ironflow/registry");
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

    // ---- remote registry ----

    #[test]
    fn parse_registry_index_valid() {
        let toml = r#"
[[templates]]
name = "hello-world"
description = "Minimal workflow handler"
category = "getting-started"
repo = "https://gitlab.com/ironflow/templates"
min_ironflow_version = "0.5.0"

[[templates]]
name = "ci-pipeline"
description = "Generic CI pipeline"
repo = "https://gitlab.com/ironflow/templates"
"#;

        let index = parse_registry_index(toml).unwrap();
        assert_eq!(index.templates.len(), 2);
        assert_eq!(index.templates[0].name, "hello-world");
        assert_eq!(
            index.templates[0].min_ironflow_version,
            Some(Version::parse("0.5.0").unwrap())
        );
        assert!(index.templates[1].min_ironflow_version.is_none());
    }

    #[test]
    fn parse_registry_index_invalid() {
        let err = parse_registry_index("not valid [[[").unwrap_err();
        assert!(err.to_string().contains("registry"));
    }

    #[test]
    fn parse_registry_index_empty() {
        let index = parse_registry_index("").unwrap();
        assert!(index.templates.is_empty());
    }

    #[test]
    fn resolve_by_name() {
        let index = RegistryIndex {
            templates: vec![
                RegistryEntry {
                    name: "hello".to_string(),
                    description: "Hello".to_string(),
                    category: None,
                    repo: "https://example.com/a".to_string(),
                    min_ironflow_version: None,
                    authors: vec![],
                    latest_version: None,
                },
                RegistryEntry {
                    name: "deploy".to_string(),
                    description: "Deploy".to_string(),
                    category: Some("ops".to_string()),
                    repo: "https://example.com/b".to_string(),
                    min_ironflow_version: None,
                    authors: vec![],
                    latest_version: None,
                },
            ],
        };

        let entry = resolve_template_entry(&index, "deploy").unwrap();
        assert_eq!(entry.repo, "https://example.com/b");
    }

    #[test]
    fn resolve_unknown_name() {
        let index = RegistryIndex {
            templates: vec![RegistryEntry {
                name: "hello".to_string(),
                description: "Hello".to_string(),
                category: None,
                repo: "https://example.com".to_string(),
                min_ironflow_version: None,
                authors: vec![],
                latest_version: None,
            }],
        };

        let err = resolve_template_entry(&index, "nope").unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("nope"), "should mention the name: {msg}");
        assert!(msg.contains("hello"), "should list available: {msg}");
    }
}
