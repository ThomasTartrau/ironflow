//! Internal helpers for `spawn_blocking`, error mapping, and serialization.

use git2::{Commit, Error, Repository, Tree};
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
