//! In-process lock serializing the operations that write to one local
//! repository.
//!
//! Two steps of a run executing in parallel, or two runs on the same worker,
//! may clone into, fetch into, add a worktree to or push from the same local
//! repository at once. git takes its own lock files and fails the second
//! caller instead of waiting, and two clones into one target race on its
//! publication. The lock makes them wait for each other instead.
//!
//! It only serializes the operations of one process: another worker, or a
//! `git` run by hand, is not covered.

use std::collections::HashMap;
use std::path::{Path, PathBuf, absolute};
use std::sync::{Arc, LazyLock, Mutex as StdMutex, PoisonError, Weak};

use tokio::fs::canonicalize;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

/// Locks of the process, keyed by the resolved repository path.
static LOCKS: LazyLock<LockRegistry> = LazyLock::new(LockRegistry::default);

/// Map from a repository path to the lock of that repository.
///
/// Entries are weak: a lock lives as long as an operation holds or waits for
/// it, and the dead entries are dropped on the next acquisition, so the map
/// does not grow with every path ever seen.
#[derive(Default)]
struct LockRegistry {
    entries: StdMutex<HashMap<PathBuf, Weak<AsyncMutex<()>>>>,
}

impl LockRegistry {
    /// Wait for the lock of the repository at `path` and return its guard.
    async fn lock(&self, path: &Path) -> OwnedMutexGuard<()> {
        let key = lock_key(path).await;
        // The std guard is released at the end of this block, before the
        // `.await` below, so the registry is never held across a suspension.
        let mutex = {
            let mut entries = self.entries.lock().unwrap_or_else(PoisonError::into_inner);
            entries.retain(|_, lock| lock.strong_count() > 0);
            match entries.get(&key).and_then(Weak::upgrade) {
                Some(mutex) => mutex,
                None => {
                    let mutex = Arc::new(AsyncMutex::new(()));
                    entries.insert(key, Arc::downgrade(&mutex));
                    mutex
                }
            }
        };
        mutex.lock_owned().await
    }
}

/// Wait for the in-process lock of the repository at `path`.
///
/// The repository is released when the returned guard is dropped. `path`
/// does not need to exist yet: a clone target and the repository it becomes
/// share the same lock.
pub(crate) async fn lock_repo(path: &Path) -> OwnedMutexGuard<()> {
    LOCKS.lock(path).await
}

/// Resolve `path` to the key of its lock, so that `dir/./repo` and
/// `dir/repo` share one, before and after the repository exists.
async fn lock_key(path: &Path) -> PathBuf {
    if let Ok(canonical) = canonicalize(path).await {
        return canonical;
    }
    if let Some(name) = path.file_name() {
        let parent = match path.parent() {
            Some(parent) if !parent.as_os_str().is_empty() => parent,
            _ => Path::new("."),
        };
        if let Ok(parent) = canonicalize(parent).await {
            return parent.join(name);
        }
    }
    match absolute(path) {
        Ok(absolute) => absolute,
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use std::fs::create_dir;
    use std::time::Duration;

    use tempfile::tempdir;
    use tokio::spawn;
    use tokio::time::timeout;

    use super::*;

    #[tokio::test]
    async fn parallel_steps_on_same_repo_are_serialized() {
        let dir = tempdir().unwrap();
        let guard = lock_repo(&dir.path().join("repo")).await;

        let same = dir.path().join(".").join("repo");
        let mut waiter = spawn(async move {
            let _guard = lock_repo(&same).await;
        });
        let still_waiting = timeout(Duration::from_millis(100), &mut waiter).await;
        assert!(
            still_waiting.is_err(),
            "second lock acquired while the first is held"
        );

        drop(guard);
        timeout(Duration::from_secs(5), waiter)
            .await
            .expect("second lock never acquired")
            .unwrap();
    }

    #[tokio::test]
    async fn different_repos_do_not_block_each_other() {
        let dir = tempdir().unwrap();
        let _a = lock_repo(&dir.path().join("a")).await;
        timeout(Duration::from_secs(5), lock_repo(&dir.path().join("b")))
            .await
            .expect("lock of another repository blocked");
    }

    #[tokio::test]
    async fn same_repo_shares_a_lock_once_it_exists() {
        let dir = tempdir().unwrap();
        let repo = dir.path().join("repo");
        let before = lock_key(&repo).await;
        create_dir(&repo).unwrap();
        assert_eq!(lock_key(&repo).await, before);
    }

    #[tokio::test]
    async fn registry_drops_unused_entries() {
        let registry = LockRegistry::default();
        let dir = tempdir().unwrap();
        drop(registry.lock(&dir.path().join("a")).await);
        drop(registry.lock(&dir.path().join("b")).await);
        let _c = registry.lock(&dir.path().join("c")).await;
        assert_eq!(registry.entries.lock().unwrap().len(), 1);
    }
}
