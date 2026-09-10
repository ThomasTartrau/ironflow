//! Parsing and functional tests for the new CLI commands:
//! run watch, run diff, init, dashboard, template create.

use std::fs;

use clap::Parser;
use ironflow_cli::cli::{Cli, Commands};
use ironflow_cli::commands;

const UUID: &str = "01234567-89ab-cdef-0123-456789abcdef";

fn parse(args: &[&str]) -> Cli {
    Cli::try_parse_from(args).unwrap()
}

// ── run watch ──────────────────────────────────────────────────

#[test]
fn parse_run_watch() {
    let cli = parse(&["ironflow-cli", "run", "watch", UUID]);
    assert!(matches!(cli.command, Commands::Run(_)));
}

#[test]
fn parse_run_watch_no_logs_and_timeout() {
    let cli = parse(&[
        "ironflow-cli",
        "run",
        "watch",
        UUID,
        "--no-logs",
        "--timeout",
        "5m",
    ]);
    assert!(matches!(cli.command, Commands::Run(_)));
}

#[test]
fn parse_run_watch_invalid_timeout() {
    assert!(
        Cli::try_parse_from(["ironflow-cli", "run", "watch", UUID, "--timeout", "abc"]).is_err()
    );
}

// ── run diff ───────────────────────────────────────────────────

#[test]
fn parse_run_diff() {
    let cli = parse(&["ironflow-cli", "run", "diff", UUID, UUID]);
    assert!(matches!(cli.command, Commands::Run(_)));
}

#[test]
fn parse_run_diff_requires_two_ids() {
    assert!(Cli::try_parse_from(["ironflow-cli", "run", "diff", UUID]).is_err());
    assert!(Cli::try_parse_from(["ironflow-cli", "run", "diff"]).is_err());
}

// ── init ───────────────────────────────────────────────────────

#[test]
fn parse_init() {
    let cli = parse(&[
        "ironflow-cli",
        "init",
        "--non-interactive",
        "--name",
        "test",
    ]);
    assert!(matches!(cli.command, Commands::Init(_)));
}

#[test]
fn parse_init_force() {
    let cli = parse(&[
        "ironflow-cli",
        "init",
        "--non-interactive",
        "--name",
        "test",
        "--force",
    ]);
    assert!(matches!(cli.command, Commands::Init(_)));
}

// ── dashboard ──────────────────────────────────────────────────

#[test]
fn parse_dashboard() {
    let cli = parse(&["ironflow-cli", "dashboard"]);
    assert!(matches!(cli.command, Commands::Dashboard(_)));
}

#[test]
fn parse_dashboard_print() {
    let cli = parse(&["ironflow-cli", "dashboard", "--print"]);
    assert!(matches!(cli.command, Commands::Dashboard(_)));
}

// ── template create ────────────────────────────────────────────

#[test]
fn parse_template_create() {
    let cli = parse(&["ironflow-cli", "template", "create", "my-template"]);
    assert!(matches!(cli.command, Commands::Template(_)));
}

#[test]
fn parse_template_create_with_output() {
    let cli = parse(&[
        "ironflow-cli",
        "template",
        "create",
        "my-template",
        "--output",
        "/tmp/tpl",
    ]);
    assert!(matches!(cli.command, Commands::Template(_)));
}

#[test]
fn parse_template_create_requires_name() {
    assert!(Cli::try_parse_from(["ironflow-cli", "template", "create"]).is_err());
}

// ── functional: template create ────────────────────────────────

#[test]
fn template_create_scaffolds_files() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dest = tmp.path().join("my-tpl");

    let args = commands::template::TemplateArgs {
        command: commands::template::TemplateCommands::Create {
            name: "my-tpl".to_string(),
            output: Some(dest.clone()),
        },
    };
    commands::template::execute(&args).unwrap();

    assert!(dest.join("template.toml").exists());
    assert!(dest.join("src/handler.rs").exists());

    let toml = fs::read_to_string(dest.join("template.toml")).unwrap();
    assert!(toml.contains(r#"name = "my-tpl""#));
    assert!(toml.contains("version = \"0.1.0\""));

    let handler = fs::read_to_string(dest.join("src/handler.rs")).unwrap();
    assert!(handler.contains("pub struct MyTpl"));
    assert!(handler.contains("fn name(&self)"));
}

#[test]
fn template_create_rejects_existing_directory() {
    let tmp = tempfile::TempDir::new().unwrap();
    let dest = tmp.path().join("existing");
    fs::create_dir_all(&dest).unwrap();

    let args = commands::template::TemplateArgs {
        command: commands::template::TemplateCommands::Create {
            name: "existing".to_string(),
            output: Some(dest),
        },
    };
    let result = commands::template::execute(&args);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("already exists"));
}
