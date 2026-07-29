//! Command-line surface: global flags, command tree, and dispatch.
//!
//! Kept in the library rather than in `main.rs` so tests can parse arbitrary
//! argument vectors -- in particular `tests/route_coverage.rs`, which checks
//! that every API route is reachable through a command that really exists.

use anyhow::Result;
use clap::{Parser, Subcommand};
use ironflow_sdk::IronflowClient;

use crate::commands;
use crate::commands::api_key::ApiKeyArgs;
use crate::commands::audit_log::AuditLogArgs;
use crate::commands::logs::LogsArgs;
use crate::commands::run::RunArgs;
use crate::commands::secret::SecretArgs;
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
}
