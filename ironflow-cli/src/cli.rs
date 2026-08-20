//! Command-line surface: global flags, command tree, and dispatch.
//!
//! Kept in the library rather than in `main.rs` so tests can parse arbitrary
//! argument vectors -- in particular `tests/route_coverage.rs`, which checks
//! that every API route is reachable through a command that really exists.

use std::io;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};
use clap_complete::Shell;
use clap_mangen::Man;
use ironflow_sdk::IronflowClient;

use crate::commands;
use crate::commands::api_key::ApiKeyArgs;
use crate::commands::audit_log::AuditLogArgs;
use crate::commands::logs::LogsArgs;
use crate::commands::run::RunArgs;
use crate::commands::secret::SecretArgs;
use crate::commands::template::TemplateArgs;
use crate::commands::user::UserArgs;
use crate::commands::workflow::WorkflowArgs;

/// CLI for the Ironflow workflow engine.
///
/// # Examples
///
/// ```
/// use clap::Parser;
/// use ironflow_cli::cli::Cli;
///
/// let cli = Cli::try_parse_from(["ironflow-cli", "run", "list"])?;
/// assert!(!cli.json);
/// # Ok::<(), clap::Error>(())
/// ```
#[derive(Debug, Parser)]
#[command(
    name = "ironflow-cli",
    version,
    about = "Drive the Ironflow workflow engine from the terminal"
)]
pub struct Cli {
    /// Output raw JSON instead of formatted tables.
    #[arg(long, global = true)]
    pub json: bool,

    /// Show verbose output (e.g. full step details in `run get`).
    #[arg(long, global = true)]
    pub verbose: bool,

    /// Override the Ironflow API base URL.
    #[arg(long, global = true, env = "IRONFLOW_URL")]
    pub url: Option<String>,

    /// Override the API key for authentication.
    #[arg(long, global = true, env = "IRONFLOW_API_KEY")]
    pub api_key: Option<String>,

    /// Command to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Manage workflow runs.
    Run(RunArgs),
    /// Manage workflows.
    Workflow(WorkflowArgs),
    /// Stream run logs via SSE.
    Logs(LogsArgs),
    /// Show global statistics.
    Stats,
    /// Manage secrets (admin only).
    Secret(SecretArgs),
    /// Manage API keys.
    #[command(name = "api-key")]
    ApiKey(ApiKeyArgs),
    /// Manage users (admin only).
    User(UserArgs),
    /// Inspect audit logs (admin only).
    #[command(name = "audit-log")]
    AuditLog(AuditLogArgs),
    /// Manage workflow templates (add, list, info).
    Template(TemplateArgs),
    /// Generate shell completions for the given shell.
    Completions {
        /// Target shell.
        shell: Shell,
    },
    /// Generate a man page and write it to stdout.
    Man,
}

/// Write shell completions for `shell` to `writer`.
///
/// # Errors
///
/// Returns an error if writing to `writer` fails.
///
/// # Examples
///
/// ```no_run
/// use ironflow_cli::cli::generate_completions;
/// use clap_complete::Shell;
///
/// let mut buf = Vec::new();
/// generate_completions(Shell::Bash, &mut buf)?;
/// assert!(!buf.is_empty());
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn generate_completions(shell: Shell, writer: &mut impl io::Write) -> Result<()> {
    let mut cmd = Cli::command();
    clap_complete::generate(shell, &mut cmd, "ironflow-cli", writer);
    Ok(())
}

/// Write a roff-formatted man page to `writer`.
///
/// # Errors
///
/// Returns an error if rendering or writing fails.
///
/// # Examples
///
/// ```no_run
/// use ironflow_cli::cli::generate_man_page;
///
/// let mut buf = Vec::new();
/// generate_man_page(&mut buf)?;
/// assert!(!buf.is_empty());
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn generate_man_page(writer: &mut impl io::Write) -> Result<()> {
    let cmd = Cli::command();
    Man::new(cmd).render(writer)?;
    Ok(())
}

/// Dispatch a parsed command against a client.
///
/// # Errors
///
/// Returns an error on API failure, invalid input, or an unconfirmed
/// destructive command.
pub async fn dispatch(client: &IronflowClient, cli: &Cli) -> Result<()> {
    match &cli.command {
        Commands::Run(args) => commands::run::execute(client, args, cli.json, cli.verbose).await,
        Commands::Workflow(args) => commands::workflow::execute(client, args, cli.json).await,
        Commands::Logs(args) => commands::logs::execute(client, args, cli.json).await,
        Commands::Stats => commands::stats::execute(client, cli.json).await,
        Commands::Secret(args) => commands::secret::execute(client, args, cli.json).await,
        Commands::ApiKey(args) => commands::api_key::execute(client, args, cli.json).await,
        Commands::User(args) => commands::user::execute(client, args, cli.json).await,
        Commands::AuditLog(args) => commands::audit_log::execute(client, args, cli.json).await,
        Commands::Template(args) => commands::template::execute(args),
        Commands::Completions { shell } => generate_completions(*shell, &mut io::stdout()),
        Commands::Man => generate_man_page(&mut io::stdout()),
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::commands::api_key::ApiKeyCommands;
    use crate::commands::audit_log::AuditLogCommands;
    use crate::commands::secret::SecretCommands;
    use crate::commands::user::UserCommands;

    use super::*;

    const UUID: &str = "01234567-89ab-cdef-0123-456789abcdef";

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(args).unwrap()
    }

    #[test]
    fn parse_run_list() {
        let cli = parse(&["ironflow-cli", "run", "list"]);
        assert!(!cli.json);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_list_with_json() {
        let cli = parse(&["ironflow-cli", "--json", "run", "list"]);
        assert!(cli.json);
    }

    #[test]
    fn parse_run_create_with_payload() {
        let cli = parse(&[
            "ironflow-cli",
            "run",
            "create",
            "deploy",
            "--payload",
            r#"{"env": "prod"}"#,
        ]);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_create_with_payload_file() {
        let cli = parse(&[
            "ironflow-cli",
            "run",
            "create",
            "deploy",
            "--payload-file",
            "/tmp/payload.json",
        ]);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_create_payload_and_file_conflict() {
        let result = Cli::try_parse_from([
            "ironflow-cli",
            "run",
            "create",
            "deploy",
            "--payload",
            "{}",
            "--payload-file",
            "/tmp/p.json",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_run_get() {
        let cli = parse(&["ironflow-cli", "run", "get", UUID]);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_cancel() {
        let cli = parse(&["ironflow-cli", "run", "cancel", UUID]);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_approve() {
        let cli = parse(&["ironflow-cli", "run", "approve", UUID]);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_reject() {
        let cli = parse(&["ironflow-cli", "run", "reject", UUID]);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_reject_requires_an_id() {
        assert!(Cli::try_parse_from(["ironflow-cli", "run", "reject"]).is_err());
    }

    #[test]
    fn parse_run_retry() {
        let cli = parse(&["ironflow-cli", "run", "retry", UUID]);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_run_list_with_filters() {
        let cli = parse(&[
            "ironflow-cli",
            "run",
            "list",
            "--status",
            "completed",
            "--workflow",
            "deploy",
            "--page",
            "2",
            "--per-page",
            "50",
        ]);
        assert!(matches!(cli.command, Commands::Run(_)));
    }

    #[test]
    fn parse_workflow_list() {
        let cli = parse(&["ironflow-cli", "workflow", "list"]);
        assert!(matches!(cli.command, Commands::Workflow(_)));
    }

    #[test]
    fn parse_workflow_get() {
        let cli = parse(&["ironflow-cli", "workflow", "get", "deploy"]);
        assert!(matches!(cli.command, Commands::Workflow(_)));
    }

    #[test]
    fn parse_logs() {
        let cli = parse(&["ironflow-cli", "logs", UUID]);
        assert!(matches!(cli.command, Commands::Logs(_)));
    }

    #[test]
    fn parse_logs_follow() {
        let cli = parse(&["ironflow-cli", "logs", UUID, "--follow"]);
        let Commands::Logs(args) = &cli.command else {
            panic!("expected Logs command");
        };
        assert!(args.follow);
    }

    #[test]
    fn parse_stats() {
        let cli = parse(&["ironflow-cli", "stats"]);
        assert!(matches!(cli.command, Commands::Stats));
    }

    #[test]
    fn parse_verbose_flag() {
        let cli = parse(&["ironflow-cli", "--verbose", "stats"]);
        assert!(cli.verbose);
    }

    #[test]
    fn parse_url_override() {
        let cli = parse(&[
            "ironflow-cli",
            "--url",
            "https://custom.example.com",
            "stats",
        ]);
        assert_eq!(cli.url.as_deref(), Some("https://custom.example.com"));
    }

    #[test]
    fn parse_invalid_uuid_rejected() {
        assert!(Cli::try_parse_from(["ironflow-cli", "run", "get", "not-a-uuid"]).is_err());
    }

    #[test]
    fn parse_no_command_fails() {
        assert!(Cli::try_parse_from(["ironflow-cli"]).is_err());
    }

    // ── Secrets ────────────────────────────────────────────────────

    #[test]
    fn parse_secret_list() {
        let cli = parse(&["ironflow-cli", "secret", "list"]);
        assert!(matches!(cli.command, Commands::Secret(_)));
    }

    #[test]
    fn parse_secret_set_with_inline_value() {
        let cli = parse(&["ironflow-cli", "secret", "set", "db/password", "hunter2"]);
        let Commands::Secret(args) = &cli.command else {
            panic!("expected Secret command");
        };
        let SecretCommands::Set { key, value } = &args.command else {
            panic!("expected Set subcommand");
        };
        assert_eq!(key, "db/password");
        assert_eq!(value.as_deref(), Some("hunter2"));
    }

    #[test]
    fn parse_secret_set_without_value_defers_to_stdin() {
        let cli = parse(&["ironflow-cli", "secret", "set", "db/password"]);
        let Commands::Secret(args) = &cli.command else {
            panic!("expected Secret command");
        };
        let SecretCommands::Set { value, .. } = &args.command else {
            panic!("expected Set subcommand");
        };
        assert!(value.is_none());
    }

    #[test]
    fn parse_secret_set_requires_a_key() {
        assert!(Cli::try_parse_from(["ironflow-cli", "secret", "set"]).is_err());
    }

    #[test]
    fn parse_secret_update() {
        let cli = parse(&["ironflow-cli", "secret", "update", "db/password", "new"]);
        assert!(matches!(cli.command, Commands::Secret(_)));
    }

    #[test]
    fn parse_secret_delete_with_yes() {
        let cli = parse(&["ironflow-cli", "secret", "delete", "db/password", "--yes"]);
        let Commands::Secret(args) = &cli.command else {
            panic!("expected Secret command");
        };
        let SecretCommands::Delete { yes, .. } = &args.command else {
            panic!("expected Delete subcommand");
        };
        assert!(yes);
    }

    #[test]
    fn parse_secret_delete_defaults_to_confirming() {
        let cli = parse(&["ironflow-cli", "secret", "delete", "db/password"]);
        let Commands::Secret(args) = &cli.command else {
            panic!("expected Secret command");
        };
        let SecretCommands::Delete { yes, .. } = &args.command else {
            panic!("expected Delete subcommand");
        };
        assert!(!yes);
    }

    #[test]
    fn parse_secret_rotate_defaults_to_the_active_version() {
        let cli = parse(&["ironflow-cli", "secret", "rotate"]);
        let Commands::Secret(args) = &cli.command else {
            panic!("expected Secret command");
        };
        let SecretCommands::Rotate(rotate) = &args.command else {
            panic!("expected Rotate subcommand");
        };
        assert!(rotate.to_version.is_none());
        assert_eq!(rotate.batch_size, 100);
    }

    #[test]
    fn parse_secret_rotate_with_version_and_batch_size() {
        let cli = parse(&[
            "ironflow-cli",
            "secret",
            "rotate",
            "--to-version",
            "2",
            "--batch-size",
            "50",
        ]);
        let Commands::Secret(args) = &cli.command else {
            panic!("expected Secret command");
        };
        let SecretCommands::Rotate(rotate) = &args.command else {
            panic!("expected Rotate subcommand");
        };
        assert_eq!(rotate.to_version, Some(2));
        assert_eq!(rotate.batch_size, 50);
    }

    #[test]
    fn parse_secret_rotate_rejects_a_non_positive_version() {
        let zero = ["ironflow-cli", "secret", "rotate", "--to-version", "0"];
        let negative = ["ironflow-cli", "secret", "rotate", "--to-version", "-1"];
        assert!(Cli::try_parse_from(zero).is_err());
        assert!(Cli::try_parse_from(negative).is_err());
    }

    #[test]
    fn parse_secret_rotate_rejects_an_out_of_range_batch_size() {
        let zero = ["ironflow-cli", "secret", "rotate", "--batch-size", "0"];
        let too_large = ["ironflow-cli", "secret", "rotate", "--batch-size", "1001"];
        assert!(Cli::try_parse_from(zero).is_err());
        assert!(Cli::try_parse_from(too_large).is_err());
    }

    #[test]
    fn parse_secret_key_status_takes_no_arguments() {
        let cli = parse(&["ironflow-cli", "secret", "key-status"]);
        let Commands::Secret(args) = &cli.command else {
            panic!("expected Secret command");
        };
        assert!(matches!(args.command, SecretCommands::KeyStatus));
        assert!(Cli::try_parse_from(["ironflow-cli", "secret", "key-status", "extra"]).is_err());
    }

    // ── API keys ───────────────────────────────────────────────────

    #[test]
    fn parse_api_key_list() {
        let cli = parse(&["ironflow-cli", "api-key", "list"]);
        assert!(matches!(cli.command, Commands::ApiKey(_)));
    }

    #[test]
    fn parse_api_key_scopes() {
        let cli = parse(&["ironflow-cli", "api-key", "scopes"]);
        assert!(matches!(cli.command, Commands::ApiKey(_)));
    }

    #[test]
    fn parse_api_key_create_with_several_scopes() {
        let cli = parse(&[
            "ironflow-cli",
            "api-key",
            "create",
            "ci",
            "--scope",
            "runs_read",
            "--scope",
            "runs_write",
        ]);
        let Commands::ApiKey(args) = &cli.command else {
            panic!("expected ApiKey command");
        };
        let ApiKeyCommands::Create { scopes, .. } = &args.command else {
            panic!("expected Create subcommand");
        };
        assert_eq!(scopes.len(), 2);
    }

    #[test]
    fn parse_api_key_create_requires_a_scope() {
        assert!(Cli::try_parse_from(["ironflow-cli", "api-key", "create", "ci"]).is_err());
    }

    #[test]
    fn parse_api_key_create_rejects_an_unknown_scope() {
        let result =
            Cli::try_parse_from(["ironflow-cli", "api-key", "create", "ci", "--scope", "root"]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_api_key_create_with_expiry() {
        let cli = parse(&[
            "ironflow-cli",
            "api-key",
            "create",
            "ci",
            "--scope",
            "admin",
            "--expires-at",
            "2026-12-31T23:59:59Z",
        ]);
        assert!(matches!(cli.command, Commands::ApiKey(_)));
    }

    #[test]
    fn parse_api_key_create_rejects_a_malformed_expiry() {
        let result = Cli::try_parse_from([
            "ironflow-cli",
            "api-key",
            "create",
            "ci",
            "--scope",
            "admin",
            "--expires-at",
            "tomorrow",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_api_key_delete_rejects_a_non_uuid() {
        assert!(Cli::try_parse_from(["ironflow-cli", "api-key", "delete", "abc"]).is_err());
    }

    // ── Users ──────────────────────────────────────────────────────

    #[test]
    fn parse_user_list() {
        let cli = parse(&["ironflow-cli", "user", "list"]);
        assert!(matches!(cli.command, Commands::User(_)));
    }

    #[test]
    fn parse_user_create() {
        let cli = parse(&[
            "ironflow-cli",
            "user",
            "create",
            "alice",
            "--email",
            "alice@example.com",
            "--password",
            "hunter2hunter2",
            "--admin",
        ]);
        let Commands::User(args) = &cli.command else {
            panic!("expected User command");
        };
        let UserCommands::Create { admin, .. } = &args.command else {
            panic!("expected Create subcommand");
        };
        assert!(admin);
    }

    #[test]
    fn parse_user_create_requires_an_email() {
        assert!(Cli::try_parse_from(["ironflow-cli", "user", "create", "alice"]).is_err());
    }

    #[test]
    fn parse_user_set_role_admin() {
        let cli = parse(&["ironflow-cli", "user", "set-role", UUID, "--admin"]);
        let Commands::User(args) = &cli.command else {
            panic!("expected User command");
        };
        let UserCommands::SetRole { admin, member, .. } = &args.command else {
            panic!("expected SetRole subcommand");
        };
        assert!(admin);
        assert!(!member);
    }

    #[test]
    fn parse_user_set_role_member() {
        let cli = parse(&["ironflow-cli", "user", "set-role", UUID, "--member"]);
        let Commands::User(args) = &cli.command else {
            panic!("expected User command");
        };
        let UserCommands::SetRole { admin, .. } = &args.command else {
            panic!("expected SetRole subcommand");
        };
        assert!(!admin);
    }

    #[test]
    fn parse_user_set_role_requires_a_role() {
        assert!(Cli::try_parse_from(["ironflow-cli", "user", "set-role", UUID]).is_err());
    }

    #[test]
    fn parse_user_set_role_rejects_both_roles() {
        let result = Cli::try_parse_from([
            "ironflow-cli",
            "user",
            "set-role",
            UUID,
            "--admin",
            "--member",
        ]);
        assert!(result.is_err());
    }

    // ── Audit logs ─────────────────────────────────────────────────

    #[test]
    fn parse_audit_log_list_without_filters() {
        let cli = parse(&["ironflow-cli", "audit-log", "list"]);
        assert!(matches!(cli.command, Commands::AuditLog(_)));
    }

    #[test]
    fn parse_audit_log_list_with_every_filter() {
        let cli = parse(&[
            "ironflow-cli",
            "audit-log",
            "list",
            "--run",
            UUID,
            "--type",
            "run_created",
            "--from",
            "2026-01-01T00:00:00Z",
            "--to",
            "2026-12-31T23:59:59Z",
            "--page",
            "2",
            "--per-page",
            "10",
        ]);
        let Commands::AuditLog(args) = &cli.command else {
            panic!("expected AuditLog command");
        };
        let AuditLogCommands::List {
            run,
            event_type,
            from,
            to,
            page,
            per_page,
        } = &args.command;
        assert!(run.is_some());
        assert!(event_type.is_some());
        assert!(from.is_some());
        assert!(to.is_some());
        assert_eq!(*page, Some(2));
        assert_eq!(*per_page, Some(10));
    }

    #[test]
    fn parse_audit_log_list_rejects_an_unknown_type() {
        let result =
            Cli::try_parse_from(["ironflow-cli", "audit-log", "list", "--type", "exploded"]);
        assert!(result.is_err());
    }

    #[test]
    fn parse_audit_log_list_rejects_a_malformed_date() {
        let result = Cli::try_parse_from(["ironflow-cli", "audit-log", "list", "--from", "hier"]);
        assert!(result.is_err());
    }

    // ── Completions & man ───────────────────────────────────────

    #[test]
    fn parse_completions_bash() {
        let cli = parse(&["ironflow-cli", "completions", "bash"]);
        let Commands::Completions { shell } = &cli.command else {
            panic!("expected Completions command");
        };
        assert_eq!(*shell, Shell::Bash);
    }

    #[test]
    fn parse_completions_zsh() {
        let cli = parse(&["ironflow-cli", "completions", "zsh"]);
        let Commands::Completions { shell } = &cli.command else {
            panic!("expected Completions command");
        };
        assert_eq!(*shell, Shell::Zsh);
    }

    #[test]
    fn parse_completions_fish() {
        let cli = parse(&["ironflow-cli", "completions", "fish"]);
        let Commands::Completions { shell } = &cli.command else {
            panic!("expected Completions command");
        };
        assert_eq!(*shell, Shell::Fish);
    }

    #[test]
    fn parse_completions_powershell() {
        let cli = parse(&["ironflow-cli", "completions", "powershell"]);
        let Commands::Completions { shell } = &cli.command else {
            panic!("expected Completions command");
        };
        assert_eq!(*shell, Shell::PowerShell);
    }

    #[test]
    fn parse_completions_requires_shell() {
        assert!(Cli::try_parse_from(["ironflow-cli", "completions"]).is_err());
    }

    #[test]
    fn parse_completions_rejects_unknown_shell() {
        assert!(Cli::try_parse_from(["ironflow-cli", "completions", "nushell"]).is_err());
    }

    #[test]
    fn parse_man() {
        let cli = parse(&["ironflow-cli", "man"]);
        assert!(matches!(cli.command, Commands::Man));
    }

    #[test]
    fn completions_bash_output_is_valid() {
        let mut buf = Vec::new();
        super::generate_completions(Shell::Bash, &mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains("ironflow-cli"));
    }

    #[test]
    fn man_page_output_is_valid() {
        let mut buf = Vec::new();
        super::generate_man_page(&mut buf).unwrap();
        let output = String::from_utf8(buf).unwrap();
        assert!(output.contains(".TH"));
        assert!(output.contains("ironflow-cli"));
    }

    // ---- template ----

    #[test]
    fn parse_template_list() {
        let cli = parse(&[
            "ironflow-cli",
            "template",
            "list",
            "https://github.com/user/templates",
        ]);
        assert!(matches!(cli.command, Commands::Template(_)));
    }

    #[test]
    fn parse_template_add() {
        let cli = parse(&[
            "ironflow-cli",
            "template",
            "add",
            "https://github.com/user/templates",
            "ci-pipeline",
        ]);
        assert!(matches!(cli.command, Commands::Template(_)));
    }

    #[test]
    fn parse_template_add_with_output() {
        let cli = parse(&[
            "ironflow-cli",
            "template",
            "add",
            "https://github.com/user/templates",
            "ci-pipeline",
            "--output",
            "my/custom/path",
        ]);
        assert!(matches!(cli.command, Commands::Template(_)));
    }

    #[test]
    fn parse_template_info() {
        let cli = parse(&[
            "ironflow-cli",
            "template",
            "info",
            "https://github.com/user/templates",
            "ci-pipeline",
        ]);
        assert!(matches!(cli.command, Commands::Template(_)));
    }

    #[test]
    fn parse_template_requires_subcommand() {
        let result = Cli::try_parse_from(["ironflow-cli", "template"]);
        assert!(result.is_err());
    }
}
