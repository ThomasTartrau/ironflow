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
use ironflow_templates::install::install_template;
use ironflow_templates::lockfile::{InstalledEntry, LOCKFILE_NAME, LockFile};
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

    if let Some(min_ver) = manifest
        .template
        .min_ironflow_version
        .as_ref()
        .filter(|_| !force)
    {
        let project_version = detect_ironflow_version()?;
        if let Err(e) = validate_ironflow_version(min_ver, &project_version) {
            anyhow::bail!("{e}\nUse --force to skip this check.");
        }
    }

    let destination = match output {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(format!("src/workflows/{name}")),
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

    // Auto-register handler
    let handler_type = to_pascal_case(name);
    let search_dirs = [
        destination.parent().unwrap_or(Path::new(".")),
        Path::new("src/workflows"),
        Path::new("src"),
    ];
    for dir in &search_dirs {
        if dir.exists() {
            let reg = detect_and_register_handler(dir, name, &handler_type)?;
            println!();
            println!("{}", reg.message);
            break;
        }
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

fn detect_ironflow_version() -> Result<Version> {
    let cargo_content = fs::read_to_string("Cargo.toml")
        .context("cannot read Cargo.toml to detect Ironflow version")?;

    let version_str = extract_dep_version(&cargo_content, "ironflow-engine").context(
        "cannot detect Ironflow version from Cargo.toml. \
         Use --force to skip the version check.",
    )?;

    Version::parse(&version_str).context(format!(
        "ironflow-engine version '{version_str}' is not valid semver. \
         Use --force to skip the version check."
    ))
}

fn extract_dep_version(content: &str, dep_name: &str) -> Option<String> {
    let doc: toml::Value = toml::from_str(content).ok()?;

    if let Some(version) = doc
        .get("dependencies")
        .and_then(|d| extract_version_value(d, dep_name))
    {
        return Some(version);
    }

    doc.get("workspace")
        .and_then(|w| w.get("dependencies"))
        .and_then(|d| extract_version_value(d, dep_name))
}

fn extract_version_value(deps: &toml::Value, name: &str) -> Option<String> {
    let dep = deps.get(name)?;
    match dep {
        toml::Value::String(s) => Some(s.clone()),
        toml::Value::Table(t) => t.get("version").and_then(|v| v.as_str()).map(String::from),
        _ => None,
    }
}

pub fn cmd_update(name: Option<&str>, check_only: bool, registry_url: Option<&str>) -> Result<()> {
    let lock_path = PathBuf::from(LOCKFILE_NAME);
    let lock = LockFile::load(&lock_path)?;

    if lock.installed().is_empty() {
        println!("No templates installed (no {LOCKFILE_NAME} found).");
        return Ok(());
    }

    let reg_url = resolve_registry_url(registry_url);
    println!("Checking registry at {reg_url}...");
    let index = fetch_registry_index(&reg_url).context("failed to fetch registry")?;

    let entries_to_check: Vec<_> = match name {
        Some(n) => {
            let entry = lock
                .find_installed(n)
                .context(format!("template '{n}' is not installed"))?;
            vec![entry]
        }
        None => lock.installed().iter().collect(),
    };

    let mut has_updates = false;

    for installed in entries_to_check {
        let registry_entry = match resolve_template_entry(&index, &installed.name) {
            Ok(e) => e,
            Err(_) => {
                println!(
                    "  {} v{} -- not found in registry (skipped)",
                    installed.name, installed.version
                );
                continue;
            }
        };

        let latest = match fetch_latest_tag(&registry_entry.repo) {
            Ok(tag) => {
                let v = tag.strip_prefix('v').unwrap_or(&tag);
                v.to_string()
            }
            Err(_) => {
                println!(
                    "  {} v{} -- no tags found in repo (skipped)",
                    installed.name, installed.version
                );
                continue;
            }
        };

        if latest == installed.version {
            println!("  {} v{} -- already up to date", installed.name, latest);
        } else {
            has_updates = true;
            println!(
                "  {} v{} -> v{} (update available)",
                installed.name, installed.version, latest
            );
            if !check_only {
                println!(
                    "    Run: ironflow-cli template add {}@{} --from {}",
                    installed.name, latest, registry_entry.repo
                );
            }
        }
    }

    if !has_updates {
        println!();
        println!("All templates are up to date.");
    }

    Ok(())
}
