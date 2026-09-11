//! `ironflow-cli template` subcommands.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Result;
use clap::{Args, Subcommand};

use ironflow_templates::fetch::fetch_repo;
use ironflow_templates::registry::{discover_templates, find_template};

mod registry_ops;

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
    /// List available templates.
    List {
        /// Local path or Git URL containing templates (omit with --registry).
        source: Option<String>,
        /// List templates from the configured registry instead.
        #[arg(long)]
        registry: bool,
        /// Registry URL (overrides config).
        #[arg(long)]
        registry_url: Option<String>,
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
        /// Template name (or name@version).
        name: String,
        /// Git URL to fetch from (bypasses registry).
        #[arg(long)]
        from: Option<String>,
        /// Use the configured registry to resolve the template name.
        #[arg(long)]
        registry: bool,
        /// Registry URL (overrides config).
        #[arg(long)]
        registry_url: Option<String>,
        /// Output directory (default: `src/workflows/<name>`).
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Skip Ironflow version compatibility check.
        #[arg(long)]
        force: bool,
    },
    /// Create a new empty template scaffold.
    Create {
        /// Template name.
        name: String,
        /// Output directory (default: `./<name>`).
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    /// Check for or apply template updates.
    Update {
        /// Template name to update (omit to check all).
        name: Option<String>,
        /// Only check for updates, do not modify files.
        #[arg(long)]
        check: bool,
        /// Registry URL (overrides config).
        #[arg(long)]
        registry_url: Option<String>,
    },
}

/// Execute a template subcommand.
///
/// # Errors
///
/// Returns an error if fetching, parsing, or installing fails.
pub fn execute(args: &TemplateArgs) -> Result<()> {
    match &args.command {
        TemplateCommands::List {
            source,
            registry,
            registry_url,
        } => {
            if *registry || source.is_none() {
                registry_ops::cmd_list_registry(registry_url.as_deref())
            } else {
                cmd_list(source.as_deref().unwrap())
            }
        }
        TemplateCommands::Info { source, name } => cmd_info(source, name),
        TemplateCommands::Add {
            name,
            from,
            registry_url,
            output,
            force,
            ..
        } => registry_ops::cmd_add(
            name,
            from.as_deref(),
            registry_url.as_deref(),
            output.as_deref(),
            *force,
        ),
        TemplateCommands::Create { name, output } => cmd_create(name, output.as_deref()),
        TemplateCommands::Update {
            name,
            check,
            registry_url,
        } => registry_ops::cmd_update(name.as_deref(), *check, registry_url.as_deref()),
    }
}

/// Resolve a source string to a local path.
pub(crate) fn resolve_source(source: &str) -> Result<ResolvedSource> {
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

pub(crate) enum ResolvedSource {
    Local(PathBuf),
    Cloned(tempfile::TempDir),
}

impl ResolvedSource {
    pub(crate) fn path(&self) -> &Path {
        match self {
            ResolvedSource::Local(p) => p,
            ResolvedSource::Cloned(tmp) => tmp.path(),
        }
    }
}

/// Parse `name@version` into `(name, Some(version))` or `(name, None)`.
pub(crate) fn parse_name_version(input: &str) -> (&str, Option<&str>) {
    match input.split_once('@') {
        Some((name, version)) => (name, Some(version)),
        None => (input, None),
    }
}

/// Convert a kebab-case name to PascalCase.
pub(crate) fn to_pascal_case(s: &str) -> String {
    s.split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                None => String::new(),
                Some(c) => c.to_uppercase().to_string() + &chars.as_str().to_lowercase(),
            }
        })
        .collect()
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
    if let Some(min_ver) = &manifest.template.min_ironflow_version {
        println!("Requires Ironflow: >= {min_ver}");
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

fn git_user_name() -> String {
    Command::new("git")
        .args(["config", "user.name"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "Author".to_string())
}

fn cmd_create(name: &str, output: Option<&Path>) -> Result<()> {
    let dest = output
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(name));

    if dest.exists() {
        anyhow::bail!("directory '{}' already exists", dest.display());
    }

    let src_dir = dest.join("src");
    fs::create_dir_all(&src_dir)?;

    let author = git_user_name();
    let template_toml = format!(
        r#"[template]
name = "{name}"
version = "0.1.0"
description = "A workflow template"
authors = ["{author}"]

[dependencies]
"#
    );
    fs::write(dest.join("template.toml"), template_toml)?;

    let handler = format!(
        r#"use ironflow_engine::context::WorkflowContext;
use ironflow_engine::handler::{{HandlerFuture, WorkflowHandler}};

pub struct {handler_name};

impl WorkflowHandler for {handler_name} {{
    fn name(&self) -> &str {{
        "{name}"
    }}

    fn execute<'a>(&'a self, ctx: &'a mut WorkflowContext) -> HandlerFuture<'a> {{
        Box::pin(async move {{
            ctx.shell("hello", "echo 'Hello from {name}!'").await?;
            Ok(())
        }})
    }}
}}
"#,
        handler_name = to_pascal_case(name)
    );
    fs::write(src_dir.join("handler.rs"), handler)?;

    println!("Template '{name}' created at {}", dest.display());

    Ok(())
}
