//! Internal helpers for `spawn_blocking`, error mapping, and serialization.

use git2::{Commit, Config, Cred, CredentialType, Error, RemoteCallbacks, Repository, Tree};
use ironflow_core::error::OperationError;
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
        message: e.message().to_string(),
    }
}

/// Build [`RemoteCallbacks`] with a `credentials` callback so libgit2 can
/// authenticate against network remotes.
///
/// Without a credentials callback, libgit2 fails on authenticated remotes with
/// `authentication required but no callback set`. The callback resolves, in
/// order of what the remote advertises:
///
/// - `SSH_KEY`: the system SSH agent (`SSH_AUTH_SOCK`), using the username from
///   the URL (or `git`), which covers `git@host:...` remotes.
/// - `USER_PASS_PLAINTEXT`: the git credential helper for HTTPS remotes.
/// - `DEFAULT`: libgit2's default credential.
pub(crate) fn credentials_callbacks<'a>() -> RemoteCallbacks<'a> {
    let mut callbacks = RemoteCallbacks::new();
    callbacks.credentials(|url, username_from_url, allowed| {
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
