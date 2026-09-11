//! Dependency injection and version validation for template installation.
//!
//! Merges template dependencies into the project's `Cargo.toml` using
//! [`toml_edit`] for format-preserving edits, and validates Ironflow
//! version compatibility using [`semver`].
//!
//! # Examples
//!
//! ```no_run
//! use std::path::Path;
//! use std::collections::HashMap;
//! use ironflow_templates::manifest::DependencySpec;
//! use ironflow_templates::deps::inject_dependencies;
//!
//! # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
//! let mut deps = HashMap::new();
//! deps.insert("serde".to_string(), DependencySpec::Simple("1".to_string()));
//! let result = inject_dependencies(Path::new("Cargo.toml"), &deps)?;
//! println!("Added: {:?}", result.added);
//! # Ok(())
//! # }
//! ```

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use semver::Version;
use toml_edit::{Array, DocumentMut, InlineTable, Item, Value};

use crate::error::TemplateError;
use crate::manifest::DependencySpec;

/// Result of dependency injection into `Cargo.toml`.
#[derive(Debug, Default)]
pub struct InjectionResult {
    /// Dependencies that were added.
    pub added: Vec<String>,
    /// Dependencies skipped because a compatible version already exists.
    pub skipped: Vec<String>,
    /// Dependencies with version conflicts (not overwritten).
    pub conflicts: Vec<String>,
}

/// Validate that the project's Ironflow version meets the template's minimum.
///
/// # Errors
///
/// Returns [`TemplateError::VersionIncompatible`] if the project version is
/// lower than the minimum.
///
/// # Examples
///
/// ```
/// use semver::Version;
/// use ironflow_templates::deps::validate_ironflow_version;
///
/// let min = Version::parse("0.5.0").unwrap();
/// let current = Version::parse("2.33.0").unwrap();
/// validate_ironflow_version(&min, &current).unwrap();
///
/// let high_min = Version::parse("3.0.0").unwrap();
/// let err = validate_ironflow_version(&high_min, &current).unwrap_err();
/// assert!(err.to_string().contains("requires Ironflow >= 3.0.0"));
/// ```
pub fn validate_ironflow_version(
    template_min: &Version,
    project_version: &Version,
) -> Result<(), TemplateError> {
    if project_version < template_min {
        return Err(TemplateError::VersionIncompatible {
            template_min: template_min.clone(),
            project_version: project_version.clone(),
        });
    }

    Ok(())
}

/// Inject template dependencies into the project's `Cargo.toml`.
///
/// Reads the existing file, merges the template's dependencies:
/// - Missing dependency: add it
/// - Existing with compatible version: skip it
/// - Existing with incompatible version: report as conflict (no overwrite)
///
/// # Errors
///
/// Returns [`TemplateError::Io`] on read/write failures, or
/// [`TemplateError::Lockfile`] on TOML parsing failures.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
/// use std::collections::HashMap;
/// use ironflow_templates::manifest::DependencySpec;
/// use ironflow_templates::deps::inject_dependencies;
///
/// # fn example() -> Result<(), ironflow_templates::error::TemplateError> {
/// let mut deps = HashMap::new();
/// deps.insert("serde".to_string(), DependencySpec::Simple("1".to_string()));
/// let result = inject_dependencies(Path::new("Cargo.toml"), &deps)?;
/// # Ok(())
/// # }
/// ```
pub fn inject_dependencies(
    cargo_toml_path: &Path,
    deps: &HashMap<String, DependencySpec>,
) -> Result<InjectionResult, TemplateError> {
    let content = fs::read_to_string(cargo_toml_path).map_err(TemplateError::Io)?;
    let mut doc: DocumentMut = content
        .parse()
        .map_err(|e| TemplateError::Lockfile(format!("failed to parse Cargo.toml: {e}")))?;

    if doc.get("dependencies").is_none() {
        doc["dependencies"] = toml_edit::table();
    }

    let deps_table = doc["dependencies"]
        .as_table_mut()
        .ok_or_else(|| TemplateError::Lockfile("[dependencies] is not a table".to_string()))?;

    let mut result = InjectionResult::default();

    for (name, spec) in deps {
        if let Some(existing) = deps_table.get(name) {
            let existing_version = extract_version_from_item(existing);
            let required_version = match spec {
                DependencySpec::Simple(v) => v.clone(),
                DependencySpec::Detailed(d) => d.version.clone(),
            };

            if versions_compatible(&existing_version, &required_version) {
                result.skipped.push(name.clone());
            } else {
                result.conflicts.push(format!(
                    "{name}: project has {existing_version}, template requires {required_version}"
                ));
            }
        } else {
            match spec {
                DependencySpec::Simple(version) => {
                    deps_table[name] = toml_edit::value(version.as_str());
                }
                DependencySpec::Detailed(d) => {
                    let mut tbl = InlineTable::new();
                    tbl.insert("version", d.version.as_str().into());
                    if !d.features.is_empty() {
                        let mut arr = Array::new();
                        for f in &d.features {
                            arr.push(f.as_str());
                        }
                        tbl.insert("features", Value::Array(arr));
                    }
                    deps_table[name] = Item::Value(Value::InlineTable(tbl));
                }
            }
            result.added.push(name.clone());
        }
    }

    result.added.sort();
    result.skipped.sort();
    result.conflicts.sort();

    if !result.added.is_empty() {
        fs::write(cargo_toml_path, doc.to_string()).map_err(TemplateError::Io)?;
    }

    Ok(result)
}

/// Extract the version string from a TOML dependency item.
fn extract_version_from_item(item: &Item) -> String {
    match item {
        Item::Value(Value::String(s)) => s.value().to_string(),
        Item::Value(Value::InlineTable(tbl)) => tbl
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        Item::Table(tbl) => tbl
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        _ => String::new(),
    }
}

/// Check if two version requirement strings are compatible.
///
/// Uses a simple heuristic: if the major versions match, they are compatible.
fn versions_compatible(existing: &str, required: &str) -> bool {
    if existing == required {
        return true;
    }

    let parse_major = |s: &str| -> Option<u64> {
        let s = s.strip_prefix('^').unwrap_or(s);
        let s = s.strip_prefix('~').unwrap_or(s);
        s.split('.').next()?.parse().ok()
    };

    match (parse_major(existing), parse_major(required)) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::manifest::DetailedDependency;

    // ---- version validation ----

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    #[test]
    fn version_compatible() {
        validate_ironflow_version(&v("0.5.0"), &v("2.33.0")).unwrap();
    }

    #[test]
    fn version_equal() {
        validate_ironflow_version(&v("2.33.0"), &v("2.33.0")).unwrap();
    }

    #[test]
    fn version_incompatible() {
        let err = validate_ironflow_version(&v("3.0.0"), &v("2.33.0")).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("3.0.0"), "should mention min: {msg}");
        assert!(msg.contains("2.33.0"), "should mention current: {msg}");
    }

    // ---- dependency injection ----

    #[test]
    fn inject_deps_adds_missing() {
        let tmp = TempDir::new().unwrap();
        let cargo_path = tmp.path().join("Cargo.toml");
        fs::write(
            &cargo_path,
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\n\n[dependencies]\n",
        )
        .unwrap();

        let mut deps = HashMap::new();
        deps.insert("serde".to_string(), DependencySpec::Simple("1".to_string()));
        deps.insert(
            "tokio".to_string(),
            DependencySpec::Detailed(DetailedDependency {
                version: "1".to_string(),
                features: vec!["full".to_string()],
            }),
        );

        let result = inject_dependencies(&cargo_path, &deps).unwrap();
        assert_eq!(result.added.len(), 2);
        assert!(result.skipped.is_empty());
        assert!(result.conflicts.is_empty());

        let content = fs::read_to_string(&cargo_path).unwrap();
        assert!(content.contains("serde"));
        assert!(content.contains("tokio"));
    }

    #[test]
    fn inject_deps_skips_compatible() {
        let tmp = TempDir::new().unwrap();
        let cargo_path = tmp.path().join("Cargo.toml");
        fs::write(
            &cargo_path,
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
        )
        .unwrap();

        let mut deps = HashMap::new();
        deps.insert("serde".to_string(), DependencySpec::Simple("1".to_string()));

        let result = inject_dependencies(&cargo_path, &deps).unwrap();
        assert!(result.added.is_empty());
        assert_eq!(result.skipped, vec!["serde"]);
        assert!(result.conflicts.is_empty());
    }

    #[test]
    fn inject_deps_warns_conflict() {
        let tmp = TempDir::new().unwrap();
        let cargo_path = tmp.path().join("Cargo.toml");
        fs::write(
            &cargo_path,
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\n\n[dependencies]\nserde = \"1\"\n",
        )
        .unwrap();

        let mut deps = HashMap::new();
        deps.insert("serde".to_string(), DependencySpec::Simple("2".to_string()));

        let result = inject_dependencies(&cargo_path, &deps).unwrap();
        assert!(result.added.is_empty());
        assert!(result.skipped.is_empty());
        assert_eq!(result.conflicts.len(), 1);
        assert!(result.conflicts[0].contains("serde"));
    }

    #[test]
    fn inject_deps_creates_dependencies_section() {
        let tmp = TempDir::new().unwrap();
        let cargo_path = tmp.path().join("Cargo.toml");
        fs::write(
            &cargo_path,
            "[package]\nname = \"test\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();

        let mut deps = HashMap::new();
        deps.insert("serde".to_string(), DependencySpec::Simple("1".to_string()));

        let result = inject_dependencies(&cargo_path, &deps).unwrap();
        assert_eq!(result.added, vec!["serde"]);
    }

    // ---- version compatibility heuristic ----

    #[test]
    fn versions_compatible_same_major() {
        assert!(versions_compatible("1", "1"));
        assert!(versions_compatible("1.5", "1.3"));
    }

    #[test]
    fn versions_compatible_different_major() {
        assert!(!versions_compatible("1", "2"));
    }

    #[test]
    fn versions_compatible_with_caret() {
        assert!(versions_compatible("^1", "1"));
    }

    #[test]
    fn versions_compatible_exact_match() {
        assert!(versions_compatible("1.2.3", "1.2.3"));
    }
}
