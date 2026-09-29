//! Internal helpers for `spawn_blocking`, error mapping, authentication,
//! URL redaction and serialization.

use std::fmt;

use git2::{Commit, Config, Cred, CredentialType, Error, RemoteCallbacks, Repository, Tree};
use ironflow_core::error::OperationError;
use ironflow_core::operation::OperationContext;
use serde::Serialize;
use serde_json::Value;
use tokio::task::spawn_blocking;

pub(crate) async fn blocking<F, T>(f: F) -> Result<T, OperationError>
where
    F: FnOnce() -> Result<T, Error> + Send + 'static,
    T: Send + 'static,
{
    spawn_blocking(f)
        .await
        .map_err(|e| OperationError::External {
            origin: "git".to_string(),
            message: e.to_string(),
        })?
        .map_err(git_error)
}

pub(crate) fn git_error(e: Error) -> OperationError {
    OperationError::External {
        origin: "git".to_string(),
        message: redact_url(e.message()),
    }
}

/// Secret read by default by network operations to authenticate over HTTPS.
pub(crate) const DEFAULT_TOKEN_SECRET: &str = "git_token";

/// Username sent with the token by default. GitLab accepts any non-empty
/// username with a personal, project or job token; `oauth2` is its convention.
pub(crate) const DEFAULT_USERNAME: &str = "oauth2";

/// Which secret, and which username, authenticate a network operation.
#[derive(Debug, Clone)]
pub(crate) struct GitAuth {
    pub(crate) token_secret: String,
    pub(crate) username: String,
}

impl Default for GitAuth {
    fn default() -> Self {
        Self {
            token_secret: DEFAULT_TOKEN_SECRET.to_string(),
            username: DEFAULT_USERNAME.to_string(),
        }
    }
}

impl GitAuth {
    /// Read the token from the secret store. `None` when the secret is
    /// absent or empty, so the operation falls back to the ambient
    /// credentials (SSH agent, credential helper).
    async fn resolve(
        &self,
        ctx: &OperationContext,
    ) -> Result<Option<GitCredentials>, OperationError> {
        let secret = ctx.secrets().get(&self.token_secret).await?;
        Ok(secret
            .filter(|s| !s.value.is_empty())
            .map(|s| GitCredentials {
                secret_key: self.token_secret.clone(),
                username: self.username.clone(),
                token: s.value,
            }))
    }
}

/// Implement `token_secret` and `username` on a network operation that holds
/// an `auth: GitAuth` field. `$module` and `$args` feed the doc example.
macro_rules! auth_builders {
    ($op:ident, $module:literal, $args:literal) => {
        impl $op {
            /// Read the HTTPS token from the secret `key` instead of `git_token`.
            ///
            /// # Examples
            ///
            #[doc = concat!(
                "```no_run\nuse ironflow_ops_git::", $module, "::", stringify!($op), ";\n\n",
                "let op = ", stringify!($op), "::new(", $args, ").token_secret(\"gitlab_token\");\n```"
            )]
            pub fn token_secret(mut self, key: impl Into<String>) -> Self {
                self.auth.token_secret = key.into();
                self
            }

            /// Send `name` as the username with the token instead of `oauth2`.
            ///
            /// # Examples
            ///
            #[doc = concat!(
                "```no_run\nuse ironflow_ops_git::", $module, "::", stringify!($op), ";\n\n",
                "let op = ", stringify!($op), "::new(", $args, ").username(\"x-access-token\");\n```"
            )]
            pub fn username(mut self, name: impl Into<String>) -> Self {
                self.auth.username = name.into();
                self
            }
        }
    };
}

pub(crate) use auth_builders;

/// A token resolved from the secret store, ready for the blocking thread.
#[derive(Clone)]
pub(crate) struct GitCredentials {
    secret_key: String,
    username: String,
    token: String,
}

impl fmt::Debug for GitCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GitCredentials")
            .field("secret_key", &self.secret_key)
            .field("username", &self.username)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

impl GitCredentials {
    /// Replace every literal occurrence of the token in an error message.
    fn scrub(&self, err: OperationError) -> OperationError {
        match err {
            OperationError::External { origin, message } => OperationError::External {
                origin,
                message: message.replace(&self.token, "***"),
            },
            other => other,
        }
    }
}

/// Run a network git call on the blocking pool, authenticated with the token
/// named by `auth`.
///
/// The secret is resolved here, before `spawn_blocking`: [`RemoteCallbacks`]
/// is not `Send`, so the callbacks are built inside `f` from the resolved
/// credentials with [`credentials_callbacks`]. The token is scrubbed from any
/// error the call returns.
pub(crate) async fn blocking_authenticated<F, T>(
    ctx: &OperationContext,
    auth: &GitAuth,
    f: F,
) -> Result<T, OperationError>
where
    F: FnOnce(Option<&GitCredentials>) -> Result<T, Error> + Send + 'static,
    T: Send + 'static,
{
    let creds = auth.resolve(ctx).await?;
    let for_call = creds.clone();
    blocking(move || f(for_call.as_ref()))
        .await
        .map_err(|e| match &creds {
            Some(c) => c.scrub(e),
            None => e,
        })
}

/// Build [`RemoteCallbacks`] with a `credentials` callback so libgit2 can
/// authenticate against network remotes.
///
/// Without a credentials callback, libgit2 fails on authenticated remotes with
/// `authentication required but no callback set`. The callback resolves, in
/// order of what the remote advertises:
///
/// - `USER_PASS_PLAINTEXT` with `creds`: the token from the secret store. If
///   the remote rejects it and asks again, the callback fails instead of
///   letting libgit2 replay the same credentials.
/// - `SSH_KEY`: the system SSH agent (`SSH_AUTH_SOCK`), using the username from
///   the URL (or `git`), which covers `git@host:...` remotes.
/// - `USER_PASS_PLAINTEXT`: the git credential helper for HTTPS remotes.
/// - `DEFAULT`: libgit2's default credential.
pub(crate) fn credentials_callbacks<'a>(creds: Option<&GitCredentials>) -> RemoteCallbacks<'a> {
    let creds = creds.cloned();
    let mut token_sent = false;
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(move |url, username_from_url, allowed| {
        if let Some(creds) = &creds
            && allowed.contains(CredentialType::USER_PASS_PLAINTEXT)
        {
            if token_sent {
                return Err(Error::from_str(&format!(
                    "the remote rejected the token from secret `{}`",
                    creds.secret_key
                )));
            }
            token_sent = true;
            return Cred::userpass_plaintext(&creds.username, &creds.token);
        }
        if allowed.contains(CredentialType::SSH_KEY) {
            return Cred::ssh_key_from_agent(username_from_url.unwrap_or("git"));
        }
        if allowed.contains(CredentialType::USER_PASS_PLAINTEXT)
            && let Ok(config) = Config::open_default()
        {
            return Cred::credential_helper(&config, url, username_from_url);
        }
        if allowed.contains(CredentialType::DEFAULT) {
            return Cred::default();
        }
        Err(Error::from_str(
            "no supported git credential type available for this remote",
        ))
    });
    callbacks
}

/// Mask the userinfo of every `scheme://user:pass@host` URL in `text`.
///
/// Works on a bare URL as well as on free text such as a libgit2 error
/// message. A username alone is masked too, since `https://TOKEN@host` is a
/// common way to pass a token. The scp-like SSH syntax (`git@host:path`) has
/// no scheme and cannot carry a password, so it is left untouched.
pub(crate) fn redact_url(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(idx) = rest.find("://") {
        let (head, tail) = rest.split_at(idx + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| {
                matches!(c, '/' | '?' | '#' | '\'' | '"' | '`' | '<' | '>') || c.is_whitespace()
            })
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        match authority.rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&authority[at..]);
            }
            None => out.push_str(authority),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

pub(crate) fn to_value<T: Serialize>(v: &T) -> Result<Value, OperationError> {
    serde_json::to_value(v).map_err(|e| OperationError::External {
        origin: "git".to_string(),
        message: e.to_string(),
    })
}

pub(crate) fn prepare_commit(repo: &Repository) -> Result<(Tree<'_>, Vec<Commit<'_>>), Error> {
    let mut index = repo.index()?;
    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;
    let parents: Vec<Commit<'_>> = repo
        .head()
        .ok()
        .and_then(|h| h.peel_to_commit().ok())
        .into_iter()
        .collect();
    Ok((tree, parents))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_error_masks_url_credentials_in_libgit2_message() {
        let err = git_error(Error::from_str(
            "failed to connect to https://oauth2:glpat-secret@gitlab.com/g/r.git",
        ));
        let message = err.to_string();
        assert!(!message.contains("glpat-secret"), "got: {message}");
        assert!(
            message.contains("https://***@gitlab.com/g/r.git"),
            "got: {message}"
        );
    }

    #[test]
    fn redact_url_masks_user_and_password() {
        assert_eq!(
            redact_url("https://oauth2:glpat-secret@gitlab.com/group/repo.git"),
            "https://***@gitlab.com/group/repo.git"
        );
    }

    #[test]
    fn redact_url_masks_username_only_token() {
        assert_eq!(
            redact_url("https://ghp_secret@github.com/org/repo.git"),
            "https://***@github.com/org/repo.git"
        );
    }

    #[test]
    fn redact_url_keeps_urls_without_userinfo() {
        for url in [
            "https://gitlab.com/group/repo.git",
            "http://127.0.0.1:8080/repo.git",
            "/tmp/local/repo",
            "file:///tmp/repo",
            "https://host/scope/@pkg",
            "",
        ] {
            assert_eq!(redact_url(url), url);
        }
    }

    #[test]
    fn redact_url_keeps_scp_like_ssh_syntax() {
        assert_eq!(
            redact_url("git@gitlab.com:group/repo.git"),
            "git@gitlab.com:group/repo.git"
        );
    }

    #[test]
    fn redact_url_masks_every_url_in_free_text() {
        assert_eq!(
            redact_url("failed to fetch 'https://a:tok1@h1/x' then ssh://u:tok2@h2:22/y"),
            "failed to fetch 'https://***@h1/x' then ssh://***@h2:22/y"
        );
    }

    #[test]
    fn redact_url_masks_url_without_path() {
        assert_eq!(redact_url("https://user:pass@host"), "https://***@host");
    }

    #[test]
    fn redact_url_handles_unicode_around_urls() {
        assert_eq!(
            redact_url("dépôt https://é:tök@hôte/ç ok"),
            "dépôt https://***@hôte/ç ok"
        );
    }
}
