//! `ironflow-cli template` subcommands.

use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Args, Subcommand};

use ironflow_templates::fetch::fetch_repo;
use ironflow_templates::install::install_template;
use ironflow_templates::registry::{discover_templates, find_template};

/// Manage workflow templates.
#[derive(Debug, Args)]
pub struct TemplateArgs {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: TemplateCommands,
}

/// Template subcommands.
#[derive(Debug, Subcommand)]
pub enum TemplateCommands {
    /// List available templates in a repository (local path or remote Git URL).
    List {
        /// Local path or Git URL containing templates.
        source: String,
    },
    /// Show details about a specific template.
    Info {
        /// Local path or Git URL containing templates.
        source: String,
        /// Template name.
        name: String,
    },
    /// Add a template from a repository into your project.
    Add {
        /// Local path or Git URL containing templates.
        source: String,
        /// Template name to install.
        name: String,
        /// Output directory (default: `src/workflows/<name>`).
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
}

/// Execute a template subcommand.
///
/// # Errors
///
/// Returns an error if fetching, parsing, or installing fails.
pub fn execute(args: &TemplateArgs) -> Result<()> {
    match &args.command {
        TemplateCommands::List { source } => cmd_list(source),
        TemplateCommands::Info { source, name } => cmd_info(source, name),
        TemplateCommands::Add {
            source,
            name,
            output,
        } => cmd_add(source, name, output.as_deref()),
    }
}

/// Resolve a source string to a local path.
///
/// If the source looks like a URL (contains `://`), clone it into a
/// temporary directory and return the path. Otherwise treat it as a
/// local path.
fn resolve_source(source: &str) -> Result<ResolvedSource> {
    if source.contains("://") {
        let tmp = fetch_repo(source)?;
        Ok(ResolvedSource::Cloned(tmp))
    } else {
        let path = PathBuf::from(source);
        if !path.exists() {
            anyhow::bail!("path does not exist: {source}");
        }
        Ok(ResolvedSource::Local(path))
    }
}

enum ResolvedSource {
    Local(PathBuf),
    Cloned(tempfile::TempDir),
}

impl ResolvedSource {
    fn path(&self) -> &Path {
        match self {
            ResolvedSource::Local(p) => p,
            ResolvedSource::Cloned(tmp) => tmp.path(),
        }
    }
}

fn cmd_list(source: &str) -> Result<()> {
    let resolved = resolve_source(source)?;
    let templates = discover_templates(resolved.path())?;

    if templates.is_empty() {
        println!("No templates found in {source}");
        return Ok(());
    }

    println!("Available templates:");
    println!();
    for (name, (manifest, _dir)) in &templates {
        println!("  {name} (v{})", manifest.template.version);
        println!("    {}", manifest.template.description);
        if let Some(cat) = &manifest.template.category {
            println!("    category: {cat}");
        }
        println!();
    }

    Ok(())
}

fn cmd_info(source: &str, name: &str) -> Result<()> {
    let resolved = resolve_source(source)?;
    let (manifest, _dir) = find_template(resolved.path(), name)?;

    println!("Template: {}", manifest.template.name);
    println!("Version:  {}", manifest.template.version);
    println!("Description: {}", manifest.template.description);

    if !manifest.template.authors.is_empty() {
        println!("Authors: {}", manifest.template.authors.join(", "));
    }
    if let Some(license) = &manifest.template.license {
        println!("License: {license}");
    }
    if let Some(cat) = &manifest.template.category {
        println!("Category: {cat}");
    }

    if !manifest.dependencies.is_empty() {
        println!();
        println!("Dependencies:");
        let mut deps: Vec<_> = manifest.dependencies.keys().collect();
        deps.sort();
        for dep in deps {
            println!("  - {dep}");
        }
    }

    Ok(())
}

fn cmd_add(source: &str, name: &str, output: Option<&Path>) -> Result<()> {
    let resolved = resolve_source(source)?;
    let (manifest, template_dir) = find_template(resolved.path(), name)?;

    let destination = match output {
        Some(path) => path.to_path_buf(),
        None => PathBuf::from(format!("src/workflows/{name}")),
    };

    let result = install_template(&manifest, &template_dir, &destination)?;

    println!(
        "Template '{name}' installed to {}",
        result.destination.display()
    );

    if !result.dependencies.is_empty() {
        println!();
        println!("Add these dependencies to your Cargo.toml:");
        println!();
        for dep in &result.dependencies {
            println!("  {dep}");
        }
    }

    Ok(())
}
