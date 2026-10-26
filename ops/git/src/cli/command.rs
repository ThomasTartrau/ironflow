//! Spawning the `git` binary with a locked-down environment.

use std::path::Path;
use std::process::Stdio;

use ironflow_core::error::OperationError;
use tokio::process::Command;

use crate::helpers::{GitCredentials, external_error, redact_url};

/// Environment variable the credential helper reads the token from.
const TOKEN_ENV: &str = "IRONFLOW_GIT_TOKEN";

/// Environment variable the credential helper reads the username from.
const USERNAME_ENV: &str = "IRONFLOW_GIT_USERNAME";

/// Inline credential helper answering `get` with the username and token held
/// in the environment of the git process, so neither is ever an argument.
/// It ignores `store` and `erase`, so nothing is persisted.
const CREDENTIAL_HELPER: &str = concat!(
    "!f() { test \"$1\" = get || return 0; ",
    "echo \"username=$IRONFLOW_GIT_USERNAME\"; ",
    "echo \"password=$IRONFLOW_GIT_TOKEN\"; }; f"
);

/// Build a `git` command, run in `repo` when set, authenticated with `creds`
/// when set.
///
/// The command never prompts (no terminal, no askpass), refuses the `ext::`
/// transport, ignores `GIT_DIR`/`GIT_WORK_TREE` inherited from the worker,
/// and is killed when dropped, so a cancelled step does not leave git
/// running. With `creds`, the inherited credential helpers are reset, so no
/// helper stores the token, the token reaches git through the environment
/// only, and repository hooks are disabled (`core.hooksPath=/dev/null`, which
/// overrides the repository configuration), since git passes its
/// environment, token included, to the hooks it runs.
pub(crate) fn git_command(creds: Option<&GitCredentials>, repo: Option<&Path>) -> Command {
    let mut cmd = Command::new("git");
    if let Some(repo) = repo {
        cmd.arg("-C").arg(repo);
    }
    cmd.args(["-c", "protocol.ext.allow=never"]);
    cmd.args(["-c", "core.askPass="]);
    if let Some(creds) = creds {
        cmd.args(["-c", "core.hooksPath=/dev/null"])
            .args(["-c", "credential.helper="])
            .arg("-c")
            .arg(format!("credential.helper={CREDENTIAL_HELPER}"))
            .env(USERNAME_ENV, creds.username())
            .env(TOKEN_ENV, creds.token());
    }
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// Run `cmd` to completion and return its trimmed stdout.
///
/// `what` names the git subcommand in the error message. The stderr of a
/// failed run has its URL credentials masked and the token of `creds`
/// scrubbed.
pub(crate) async fn run_git(
    mut cmd: Command,
    what: &str,
    creds: Option<&GitCredentials>,
) -> Result<String, OperationError> {
    let output = cmd
        .output()
        .await
        .map_err(|e| external_error(format!("failed to spawn git: {e}")))?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).trim().to_string());
    }
    let code = match output.status.code() {
        Some(code) => code.to_string(),
        None => "none (killed by a signal)".to_string(),
    };
    let stderr = String::from_utf8_lossy(&output.stderr);
    let err = external_error(redact_url(&format!(
        "git {what} failed with exit code {code}: {}",
        stderr.trim()
    )));
    Err(match creds {
        Some(creds) => creds.scrub(err),
        None => err,
    })
}

/// Check that `name` is a valid branch name, `kind` naming it in the error.
///
/// # Errors
///
/// Returns [`OperationError::External`] for an empty name, a name starting
/// with `-`, or a name `git check-ref-format` rejects.
pub(crate) async fn validate_ref_name(kind: &str, name: &str) -> Result<(), OperationError> {
    let invalid = || external_error(format!("invalid {kind} name {name:?}"));
    if name.is_empty() || name.starts_with('-') {
        return Err(invalid());
    }
    let mut cmd = git_command(None, None);
    let full = format!("refs/heads/{name}");
    cmd.args(["check-ref-format", full.as_str()]);
    run_git(cmd, "check-ref-format", None)
        .await
        .map_err(|e| external_error(format!("{}: {e}", invalid())))?;
    Ok(())
}

/// Check that `remote` can only be read as a remote name or URL by git.
///
/// # Errors
///
/// Returns [`OperationError::External`] for an empty name, a name starting
/// with `-`, a name holding a control character, or a transport-helper form
/// (`<transport>::<address>`).
pub(crate) fn validate_remote_name(remote: &str) -> Result<(), OperationError> {
    if remote.is_empty()
        || remote.starts_with('-')
        || remote.contains("::")
        || remote.chars().any(char::is_control)
    {
        return Err(external_error(format!("invalid remote name {remote:?}")));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;
    use crate::helpers::GitAuth;
    use crate::test_helpers::ctx;

    const TOKEN: &str = "glpat-command-test-token";

    async fn creds(token: &str) -> GitCredentials {
        let auth = GitAuth {
            token: Some(token.into()),
            ..Default::default()
        };
        auth.resolve(&ctx()).await.unwrap().unwrap()
    }

    fn args(cmd: &Command) -> Vec<String> {
        let mut args = Vec::new();
        for arg in cmd.as_std().get_args() {
            args.push(arg.to_string_lossy().into_owned());
        }
        args
    }

    fn env_value(cmd: &Command, key: &str) -> Option<String> {
        for (k, v) in cmd.as_std().get_envs() {
            if k == OsStr::new(key) {
                return v.map(|v| v.to_string_lossy().into_owned());
            }
        }
        None
    }

    #[tokio::test]
    async fn token_reaches_git_through_env_not_argv() {
        let creds = creds(TOKEN).await;
        let cmd = git_command(Some(&creds), None);
        let args = args(&cmd);
        assert!(!args.iter().any(|a| a.contains(TOKEN)), "token in argv");
        assert_eq!(env_value(&cmd, TOKEN_ENV).as_deref(), Some(TOKEN));
        assert_eq!(env_value(&cmd, USERNAME_ENV).as_deref(), Some("oauth2"));
        assert!(args.contains(&"credential.helper=".to_string()));
        assert!(args.iter().any(|a| a.starts_with("credential.helper=!")));
        assert!(args.contains(&"core.hooksPath=/dev/null".to_string()));
    }

    #[test]
    fn no_credential_override_without_token() {
        let cmd = git_command(None, None);
        let args = args(&cmd);
        assert!(!args.iter().any(|a| a.contains("credential.helper")));
        assert_eq!(env_value(&cmd, TOKEN_ENV), None);
        let prompt = env_value(&cmd, "GIT_TERMINAL_PROMPT");
        assert_eq!(prompt.as_deref(), Some("0"));
    }

    #[tokio::test]
    async fn failed_git_error_masks_url_credentials_and_token() {
        let creds = creds("SECRET").await;
        let mut cmd = git_command(Some(&creds), None);
        cmd.args(["ls-remote", "http://u:SECRET@127.0.0.1:1/x"]);
        let err = run_git(cmd, "ls-remote", Some(&creds)).await.unwrap_err();
        let message = err.to_string();
        assert!(!message.contains("SECRET"), "got: {message}");
        assert!(message.contains("git ls-remote failed"), "got: {message}");
    }

    #[tokio::test]
    async fn empty_explicit_token_is_rejected() {
        let auth = GitAuth {
            token: Some(String::new()),
            ..Default::default()
        };
        let message = auth.resolve(&ctx()).await.unwrap_err().to_string();
        assert!(message.contains("empty"), "got: {message}");
    }

    #[tokio::test]
    async fn explicit_token_is_redacted_in_debug() {
        let auth = GitAuth {
            token: Some(TOKEN.into()),
            ..Default::default()
        };
        assert!(!format!("{auth:?}").contains(TOKEN));
        assert!(!format!("{:?}", creds(TOKEN).await).contains(TOKEN));
    }

    #[tokio::test]
    async fn invalid_ref_names_are_rejected() {
        for name in ["", "-bad", "a..b", "a b", "x~1"] {
            let result = validate_ref_name("branch", name).await;
            assert!(result.is_err(), "accepted: {name:?}");
        }
        validate_ref_name("branch", "feature/ok-1").await.unwrap();
    }

    #[test]
    fn invalid_remote_names_are_rejected() {
        for name in ["", "-o", "ext::sh", "a\nb"] {
            assert!(validate_remote_name(name).is_err(), "accepted: {name:?}");
        }
        validate_remote_name("origin").unwrap();
    }
}
