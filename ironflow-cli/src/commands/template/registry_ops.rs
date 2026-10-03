//! Registry-aware template operations: add with resolution, list from
//! registry, and update checking.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use semver::Version;

use ironflow_templates::auto_register::detect_and_register_handler;
use ironflow_templates::deps::inject_dependencies;
use ironflow_templates::deps::{InjectionResult, validate_ironflow_version};
use ironflow_templates::fetch::{fetch_latest_tag, fetch_repo_at_tag, validate_template_name};
use ironflow_templates::install::{default_destination, install_template, replace_template};
use ironflow_templates::lockfile::{InstalledEntry, LOCKFILE_NAME, LockFile};
use ironflow_templates::manifest::TemplateManifest;
use ironflow_templates::registry::{
    fetch_registry_index, find_template, resolve_registry_url, resolve_template_entry,
};

use super::{ResolvedSource, parse_name_version, resolve_source, to_pascal_case};

pub fn cmd_list_registry(registry_url: Option<&str>) -> Result<()> {
    let url = resolve_registry_url(registry_url);
    println!("Fetching registry from {url}...");

    let index = fetch_registry_index(&url).context("failed to fetch registry index")?;

    if index.templates.is_empty() {
        println!("No templates available in the registry.");
        return Ok(());
    }

    println!();
    println!("Available templates:");
    println!();
    for entry in &index.templates {
        let version_info = entry
            .min_ironflow_version
            .as_ref()
            .map(|v| format!(" (requires >= {v})"))
            .unwrap_or_default();
        let category = entry
            .category
            .as_deref()
            .map(|c| format!(" [{c}]"))
            .unwrap_or_default();

        println!("  {}{category}{version_info}", entry.name);
        println!("    {}", entry.description);
        println!();
    }

    Ok(())
}

pub fn cmd_add(
    name_input: &str,
    from: Option<&str>,
    registry_url: Option<&str>,
    output: Option<&Path>,
    force: bool,
) -> Result<()> {
    let (name, version) = parse_name_version(name_input);
    validate_template_name(name)?;
    let (resolved, repo_url) = resolve_add_source(name, version, from, registry_url)?;
    let (manifest, template_dir) = find_template(resolved.path(), name)?;

    check_min_version(&manifest, force)?;

    let destination = match output {
        Some(path) => path.to_path_buf(),
        None => default_destination(Path::new("."), name),
    };

    let result = install_template(&manifest, &template_dir, &destination)?;
    println!(
        "Template '{name}' installed to {}",
        result.destination.display()
    );

    // Auto-inject dependencies
    let cargo_toml = PathBuf::from("Cargo.toml");
    if cargo_toml.exists() && !manifest.dependencies.is_empty() {
        let injection = inject_dependencies(&cargo_toml, &manifest.dependencies)?;
        print_injection_result(&injection);
    } else if !result.dependencies.is_empty() {
        println!();
        println!("Add these dependencies to your Cargo.toml:");
        for dep in &result.dependencies {
            println!("  {dep}");
        }
    }

    // Auto-register handler. Only the directory holding the installed module
    // can declare it: a `mod` elsewhere would not find the files.
    let handler_type = to_pascal_case(name);
    let module_parent = destination.parent().unwrap_or(Path::new("."));
    let reg = detect_and_register_handler(module_parent, name, &handler_type)?;
    println!();
    println!("{}", reg.message);

    let requirements = manifest.requirements.render();
    if !requirements.is_empty() {
        println!();
        print!("{requirements}");
    }

    // Update lockfile
    let lock_path = PathBuf::from(LOCKFILE_NAME);
    let mut lock = LockFile::load(&lock_path)?;
    lock.record_install(InstalledEntry {
        name: name.to_string(),
        version: manifest.template.version.clone(),
        repo: repo_url,
        installed_at: chrono::Utc::now().format("%Y-%m-%d").to_string(),
        path: destination.display().to_string(),
    });
    lock.save(&lock_path)?;
    println!();
    println!("Lockfile updated: {LOCKFILE_NAME}");

    Ok(())
}

fn print_injection_result(injection: &InjectionResult) {
    if !injection.added.is_empty() {
        println!();
        println!("Dependencies added to Cargo.toml:");
        for dep in &injection.added {
            println!("  + {dep}");
        }
    }
    if !injection.skipped.is_empty() {
        println!();
        println!("Dependencies already present (skipped):");
        for dep in &injection.skipped {
            println!("  = {dep}");
        }
    }
    if !injection.conflicts.is_empty() {
        println!();
        println!("Version conflicts (not modified):");
        for conflict in &injection.conflicts {
            println!("  ! {conflict}");
        }
    }
}

fn resolve_add_source(
    name: &str,
    version: Option<&str>,
    from: Option<&str>,
    registry_url: Option<&str>,
) -> Result<(ResolvedSource, String)> {
    if let Some(url) = from {
        let resolved = if let Some(tag) = version {
            let tag = format_tag(tag);
            ResolvedSource::Cloned(fetch_repo_at_tag(url, &tag)?)
        } else {
            resolve_source(url)?
        };
        return Ok((resolved, url.to_string()));
    }

    let reg_url = resolve_registry_url(registry_url);
    let index = fetch_registry_index(&reg_url)
        .context("failed to fetch registry -- use --from <url> to bypass")?;
    let entry = resolve_template_entry(&index, name)?;
    let repo_url = entry.repo.clone();

    let resolved = if let Some(tag) = version {
        let tag = format_tag(tag);
        ResolvedSource::Cloned(fetch_repo_at_tag(&repo_url, &tag)?)
    } else {
        match fetch_latest_tag(&repo_url) {
            Ok(tag) => ResolvedSource::Cloned(fetch_repo_at_tag(&repo_url, &tag)?),
            Err(_) => resolve_source(&repo_url)?,
        }
    };

    Ok((resolved, repo_url))
}

fn format_tag(version: &str) -> String {
    if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    }
}

const IRONFLOW_DEP: &str = "ironflow-engine";

/// Where the version of a dependency is declared.
enum DepSource {
    /// Written in the manifest that declares the dependency.
    Version(String),
    /// Inherited with `workspace = true`, declared in the workspace root.
    Workspace,
}

/// Check the project's Ironflow version against the template's minimum.
fn check_min_version(manifest: &TemplateManifest, force: bool) -> Result<()> {
    let Some(min_ver) = manifest
        .template
        .min_ironflow_version
        .as_ref()
        .filter(|_| !force)
    else {
        return Ok(());
    };

    let project_version = detect_ironflow_version_in(Path::new("."))?;
    if let Err(e) = validate_ironflow_version(min_ver, &project_version) {
        anyhow::bail!("{e}\nUse --force to skip this check.");
    }
    Ok(())
}

/// Read the `ironflow-engine` version of the project in `dir`, following
/// `workspace = true` up to the workspace root.
fn detect_ironflow_version_in(dir: &Path) -> Result<Version> {
    let cargo_content = fs::read_to_string(dir.join("Cargo.toml"))
        .context("cannot read Cargo.toml to detect Ironflow version")?;

    let version_str = match dep_source(&cargo_content, IRONFLOW_DEP) {
        Some(DepSource::Version(v)) => Some(v),
        Some(DepSource::Workspace) => workspace_dep_version(dir, IRONFLOW_DEP)?,
        None => None,
    }
    .context(
        "cannot detect Ironflow version from Cargo.toml. \
         Use --force to skip the version check.",
    )?;

    Version::parse(&version_str).context(format!(
        "ironflow-engine version '{version_str}' is not valid semver. \
         Use --force to skip the version check."
    ))
}

fn dep_source(content: &str, dep_name: &str) -> Option<DepSource> {
    let doc: toml::Value = toml::from_str(content).ok()?;
    let workspace_deps = doc.get("workspace").and_then(|w| w.get("dependencies"));

    if let Some(deps) = doc.get("dependencies") {
        let inherits = deps
            .get(dep_name)
            .and_then(|d| d.get("workspace"))
            .and_then(toml::Value::as_bool)
            == Some(true);
        if inherits {
            // A root package inheriting from its own `[workspace]` table.
            return match workspace_deps.and_then(|d| extract_version_value(d, dep_name)) {
                Some(version) => Some(DepSource::Version(version)),
                None => Some(DepSource::Workspace),
            };
        }
        if let Some(version) = extract_version_value(deps, dep_name) {
            return Some(DepSource::Version(version));
        }
    }

    workspace_deps
        .and_then(|d| extract_version_value(d, dep_name))
        .map(DepSource::Version)
}

/// Version declared in `[workspace.dependencies]` of the nearest workspace
/// root above `dir`.
fn workspace_dep_version(dir: &Path, dep_name: &str) -> Result<Option<String>> {
    let start = dir
        .canonicalize()
        .context("cannot resolve the project directory to find its workspace root")?;

    for ancestor in start.ancestors().skip(1) {
        let Ok(content) = fs::read_to_string(ancestor.join("Cargo.toml")) else {
            continue;
        };
        let Ok(doc) = toml::from_str::<toml::Value>(&content) else {
            continue;
        };
        if let Some(workspace) = doc.get("workspace") {
            return Ok(workspace
                .get("dependencies")
                .and_then(|d| extract_version_value(d, dep_name)));
        }
    }
    Ok(None)
}

fn extract_version_value(deps: &toml::Value, name: &str) -> Option<String> {
    let dep = deps.get(name)?;
    match dep {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Table(t) => t.get("version").and_then(|v| v.as_str()).map(String::from),
        _ => None,
    }
}

/// Whether `latest` is a strictly newer release than `installed`.
fn is_newer(latest: &Version, installed: &Version) -> bool {
    latest > installed
}

pub fn cmd_update(
    name: Option<&str>,
    check_only: bool,
    force: bool,
    _registry_url: Option<&str>,
) -> Result<()> {
    let lock_path = PathBuf::from(LOCKFILE_NAME);
    let mut lock = LockFile::load(&lock_path)?;

    if lock.installed().is_empty() {
        println!("No templates installed (no {LOCKFILE_NAME} found).");
        return Ok(());
    }

    // Updates come from the repository recorded at install time, so a template
    // added with `--from` is never replaced by a same-named registry one.

    let entries_to_check: Vec<InstalledEntry> = match name {
        Some(n) => {
            let entry = lock
                .find_installed(n)
                .context(format!("template '{n}' is not installed"))?;
            vec![entry.clone()]
        }
        None => lock.installed().to_vec(),
    };

    let mut has_updates = false;

    for installed in entries_to_check {
        let latest_tag = match fetch_latest_tag(&installed.repo) {
            Ok(tag) => tag,
            Err(_) => {
                println!(
                    "  {} v{} -- no tags found in repo (skipped)",
                    installed.name, installed.version
                );
                continue;
            }
        };

        let latest_str = latest_tag.strip_prefix('v').unwrap_or(&latest_tag);
        let (Ok(latest), Ok(current)) = (
            Version::parse(latest_str),
            Version::parse(&installed.version),
        ) else {
            println!(
                "  {} v{} -- invalid version (skipped)",
                installed.name, installed.version
            );
            continue;
        };

        if !is_newer(&latest, &current) {
            println!("  {} v{} -- already up to date", installed.name, current);
            continue;
        }

        has_updates = true;
        if check_only {
            println!(
                "  {} v{} -> v{} (update available)",
                installed.name, current, latest
            );
        } else {
            apply_update(&installed, &latest, force, &mut lock)?;
        }
    }

    if !has_updates {
        println!();
        println!("All templates are up to date.");
    }

    Ok(())
}

/// Replace the installed copy of a template with the version tagged `latest`.
///
/// Dependencies are injected before the swap and the lockfile write, so a
/// failure leaves the lockfile on the old version and the next run retries.
fn apply_update(
    installed: &InstalledEntry,
    latest: &Version,
    force: bool,
    lock: &mut LockFile,
) -> Result<()> {
    let destination = PathBuf::from(&installed.path);
    if !destination.is_dir() {
        anyhow::bail!(
            "installed path '{}' not found; reinstall with template add",
            installed.path
        );
    }

    let tmp = fetch_repo_at_tag(&installed.repo, &format_tag(&latest.to_string()))?;
    let (manifest, template_dir) = find_template(tmp.path(), &installed.name)?;
    check_min_version(&manifest, force)?;

    let cargo_toml = PathBuf::from("Cargo.toml");
    let injection = if cargo_toml.exists() && !manifest.dependencies.is_empty() {
        Some(inject_dependencies(&cargo_toml, &manifest.dependencies)?)
    } else {
        None
    };

    let result = replace_template(&manifest, &template_dir, &destination)?;

    lock.record_install(InstalledEntry {
        name: installed.name.clone(),
        version: manifest.template.version.clone(),
        repo: installed.repo.clone(),
        installed_at: chrono::Utc::now().format("%Y-%m-%d").to_string(),
        path: installed.path.clone(),
    });
    lock.save(Path::new(LOCKFILE_NAME))?;

    println!(
        "  {} v{} -> v{} updated",
        installed.name, installed.version, manifest.template.version
    );
    for file in &result.added {
        println!("    + {file}");
    }
    for file in &result.modified {
        println!("    ~ {file}");
    }
    for file in &result.removed {
        println!("    - {file}");
    }

    if let Some(injection) = &injection {
        print_injection_result(injection);
    }

    let requirements = manifest.requirements.render();
    if !requirements.is_empty() {
        println!();
        print!("{requirements}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    const WORKSPACE_ROOT: &str = "[workspace]\nmembers = [\"crates/app\"]\n\n[workspace.dependencies]\nironflow-engine = { version = \"0.1.5\" }\n";

    fn write_member(root: &Path, manifest: &str) -> PathBuf {
        let member = root.join("crates").join("app");
        fs::create_dir_all(&member).unwrap();
        fs::write(member.join("Cargo.toml"), manifest).unwrap();
        member
    }

    #[test]
    fn detect_reads_direct_version() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"p\"\n\n[dependencies]\nironflow-engine = \"0.1.2\"\n",
        )
        .unwrap();

        let version = detect_ironflow_version_in(tmp.path()).unwrap();

        assert_eq!(version, Version::new(0, 1, 2));
    }

    #[test]
    fn detect_follows_workspace_true_to_grandparent_root() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), WORKSPACE_ROOT).unwrap();
        let member = write_member(
            tmp.path(),
            "[package]\nname = \"app\"\n\n[dependencies]\nironflow-engine = { workspace = true }\n",
        );

        let version = detect_ironflow_version_in(&member).unwrap();

        assert_eq!(version, Version::new(0, 1, 5));
    }

    #[test]
    fn detect_follows_dotted_workspace_form() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("Cargo.toml"), WORKSPACE_ROOT).unwrap();
        let member = write_member(
            tmp.path(),
            "[package]\nname = \"app\"\n\n[dependencies]\nironflow-engine.workspace = true\n",
        );

        let version = detect_ironflow_version_in(&member).unwrap();

        assert_eq!(version, Version::new(0, 1, 5));
    }

    #[test]
    fn detect_fails_without_workspace_root() {
        let tmp = TempDir::new().unwrap();
        let member = write_member(
            tmp.path(),
            "[package]\nname = \"app\"\n\n[dependencies]\nironflow-engine = { workspace = true }\n",
        );

        let err = detect_ironflow_version_in(&member).unwrap_err();

        assert!(err.to_string().contains("Use --force"));
    }

    #[test]
    fn detect_fails_when_root_lacks_the_dependency() {
        let tmp = TempDir::new().unwrap();
        fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"crates/app\"]\n",
        )
        .unwrap();
        let member = write_member(
            tmp.path(),
            "[package]\nname = \"app\"\n\n[dependencies]\nironflow-engine = { workspace = true }\n",
        );

        let err = detect_ironflow_version_in(&member).unwrap_err();

        assert!(err.to_string().contains("Use --force"));
    }

    #[test]
    fn is_newer_compares_semver_not_strings() {
        let v = |s: &str| Version::parse(s).unwrap();

        assert!(is_newer(&v("0.10.0"), &v("0.9.0")));
        assert!(!is_newer(&v("0.2.0"), &v("0.2.0")));
        assert!(!is_newer(&v("0.2.0"), &v("0.3.0")));
    }
}
