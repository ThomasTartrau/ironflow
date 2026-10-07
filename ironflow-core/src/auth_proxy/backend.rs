//! Where the proxy keeps its grants.

use std::collections::HashMap;
use std::future::{Future, ready};
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};

use super::AuthProxyError;
use super::registry::Grant;

/// Boxed future returned by every [`GrantBackend`] method.
pub type GrantFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, AuthProxyError>> + Send + 'a>>;

/// Storage of the grants behind an [`AuthProxyRegistry`](super::AuthProxyRegistry).
///
/// Grants are keyed by their [`token_id`](super::token_id): the opaque token
/// itself is never passed to a backend, so it can never store it. An
/// implementation keeping grants outside the process (a database) must
/// encrypt the credential at rest.
///
/// # Examples
///
/// ```
/// use std::sync::Arc;
///
/// use ironflow_core::auth_proxy::{AuthProxyRegistry, GrantBackend, MemoryGrantBackend};
///
/// let backend: Arc<dyn GrantBackend> = Arc::new(MemoryGrantBackend::default());
/// let registry = AuthProxyRegistry::with_backend(backend);
/// # let _ = registry;
/// ```
pub trait GrantBackend: Send + Sync {
    /// Store `grant` under its [`Grant::id`].
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails.
    fn insert(&self, grant: Grant) -> GrantFuture<'_, ()>;

    /// The grant stored under `id`, if any. Expiry is not checked here.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails or the
    /// stored grant cannot be read back.
    fn get(&self, id: &str) -> GrantFuture<'_, Option<Grant>>;

    /// Drop the grant stored under `id`, without remembering it: used once
    /// the grant has expired. Returns whether it existed.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails.
    fn remove(&self, id: &str) -> GrantFuture<'_, bool>;

    /// Revoke the grant stored under `id`: drop it (and so its credential)
    /// and remember its id, run and expiry until
    /// [`purge_expired`](GrantBackend::purge_expired) passes that expiry.
    /// Returns `false`, remembering nothing, when no grant is stored.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails.
    fn revoke(&self, id: &str) -> GrantFuture<'_, bool>;

    /// Revoke every grant of `run_id`, as [`revoke`](GrantBackend::revoke)
    /// does for one. Returns how many were revoked.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails.
    fn revoke_run(&self, run_id: &str) -> GrantFuture<'_, usize>;

    /// Whether `id` was revoked and its expiry has not been purged yet.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails.
    fn is_revoked(&self, id: &str) -> GrantFuture<'_, bool>;

    /// Drop every grant with `expires_at <= now`, and every revocation with
    /// `expires_at <= now`. Returns how many grants were dropped.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails.
    fn purge_expired(&self, now: u64) -> GrantFuture<'_, usize>;

    /// Number of stored grants. Revocations are not counted.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails.
    fn len(&self) -> GrantFuture<'_, usize>;

    /// Whether no grant is stored.
    ///
    /// # Errors
    ///
    /// Returns [`AuthProxyError::Backend`] when the storage fails.
    fn is_empty(&self) -> GrantFuture<'_, bool> {
        Box::pin(async move { Ok(self.len().await? == 0) })
    }
}

/// In-process [`GrantBackend`]: the default one. Grants are lost on restart
/// and are not shared between replicas. Cheap to clone: clones share the
/// same grants.
///
/// # Examples
///
/// ```
/// use ironflow_core::auth_proxy::{
///     CredentialKind, Grant, GrantBackend, MemoryGrantBackend, ProxyCredential,
/// };
///
/// # async fn example() -> Result<(), ironflow_core::auth_proxy::AuthProxyError> {
/// let backend = MemoryGrantBackend::default();
/// backend
///     .insert(Grant {
///         run_id: "run-1".to_string(),
///         step: "review".to_string(),
///         expires_at: 200,
///         credential: ProxyCredential::new(CredentialKind::ApiKey, "sk-ant-api03-x".to_string())
///             .into(),
///         id: "abc".to_string(),
///     })
///     .await?;
/// assert_eq!(backend.len().await?, 1);
/// assert!(backend.remove("abc").await?);
/// # Ok(())
/// # }
/// ```
#[derive(Clone, Default)]
pub struct MemoryGrantBackend {
    grants: Arc<Mutex<HashMap<String, Grant>>>,
    revoked: Arc<Mutex<HashMap<String, u64>>>,
}

impl MemoryGrantBackend {
    fn grants(&self) -> MutexGuard<'_, HashMap<String, Grant>> {
        self.grants.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn revoked(&self) -> MutexGuard<'_, HashMap<String, u64>> {
        self.revoked.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl GrantBackend for MemoryGrantBackend {
    fn insert(&self, grant: Grant) -> GrantFuture<'_, ()> {
        self.grants().insert(grant.id.clone(), grant);
        Box::pin(ready(Ok(())))
    }

    fn get(&self, id: &str) -> GrantFuture<'_, Option<Grant>> {
        let grant = self.grants().get(id).cloned();
        Box::pin(ready(Ok(grant)))
    }

    fn remove(&self, id: &str) -> GrantFuture<'_, bool> {
        let removed = self.grants().remove(id).is_some();
        Box::pin(ready(Ok(removed)))
    }

    fn revoke(&self, id: &str) -> GrantFuture<'_, bool> {
        let removed = self.grants().remove(id);
        let revoked = match removed {
            Some(grant) => {
                self.revoked().insert(grant.id, grant.expires_at);
                true
            }
            None => false,
        };
        Box::pin(ready(Ok(revoked)))
    }

    fn revoke_run(&self, run_id: &str) -> GrantFuture<'_, usize> {
        let removed: Vec<Grant> = {
            let mut grants = self.grants();
            let ids: Vec<String> = grants
                .values()
                .filter(|grant| grant.run_id == run_id)
                .map(|grant| grant.id.clone())
                .collect();
            ids.iter().filter_map(|id| grants.remove(id)).collect()
        };
        let count = removed.len();
        let mut revoked = self.revoked();
        for grant in removed {
            revoked.insert(grant.id, grant.expires_at);
        }
        Box::pin(ready(Ok(count)))
    }

    fn is_revoked(&self, id: &str) -> GrantFuture<'_, bool> {
        let revoked = self.revoked().contains_key(id);
        Box::pin(ready(Ok(revoked)))
    }

    fn purge_expired(&self, now: u64) -> GrantFuture<'_, usize> {
        let purged = {
            let mut grants = self.grants();
            let before = grants.len();
            grants.retain(|_, grant| grant.expires_at > now);
            before - grants.len()
        };
        self.revoked().retain(|_, expires_at| *expires_at > now);
        Box::pin(ready(Ok(purged)))
    }

    fn len(&self) -> GrantFuture<'_, usize> {
        let len = self.grants().len();
        Box::pin(ready(Ok(len)))
    }
}

#[cfg(test)]
mod tests {
    use super::super::registry::{AuthProxyRegistry, TokenRequest, token_id};
    use super::super::{CredentialKind, ProxyCredential};
    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn grant(id: &str, run_id: &str, expires_at: u64) -> Grant {
        Grant {
            run_id: run_id.to_string(),
            step: "review".to_string(),
            expires_at,
            credential: ProxyCredential::new(
                CredentialKind::OauthToken,
                "sk-ant-oat01-test".to_string(),
            )
            .into(),
            id: id.to_string(),
        }
    }

    #[tokio::test]
    async fn insert_then_get_returns_grant() {
        let backend = MemoryGrantBackend::default();
        backend.insert(grant("a", "run-1", NOW)).await.unwrap();
        let stored = backend.get("a").await.unwrap().unwrap();
        assert_eq!(stored.id, "a");
        assert_eq!(stored.run_id, "run-1");
        assert_eq!(stored.expires_at, NOW);
        assert_eq!(stored.credential.expose(), "sk-ant-oat01-test");
        assert!(backend.get("b").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn remove_reports_whether_it_existed() {
        let backend = MemoryGrantBackend::default();
        backend.insert(grant("a", "run-1", NOW)).await.unwrap();
        assert!(!backend.is_empty().await.unwrap());
        assert!(backend.remove("a").await.unwrap());
        assert!(!backend.remove("a").await.unwrap());
        assert_eq!(backend.len().await.unwrap(), 0);
        assert!(backend.is_empty().await.unwrap());
    }

    #[tokio::test]
    async fn revoke_run_only_drops_that_run() {
        let backend = MemoryGrantBackend::default();
        backend.insert(grant("a1", "run-a", NOW)).await.unwrap();
        backend.insert(grant("a2", "run-a", NOW)).await.unwrap();
        backend.insert(grant("b", "run-b", NOW)).await.unwrap();
        assert_eq!(backend.revoke_run("run-a").await.unwrap(), 2);
        assert_eq!(backend.revoke_run("run-a").await.unwrap(), 0);
        assert_eq!(backend.len().await.unwrap(), 1);
        assert!(backend.get("b").await.unwrap().is_some());
        assert!(backend.is_revoked("a1").await.unwrap());
        assert!(backend.is_revoked("a2").await.unwrap());
        assert!(!backend.is_revoked("b").await.unwrap());
    }

    #[tokio::test]
    async fn revoke_drops_the_grant_and_keeps_a_tombstone() {
        let backend = MemoryGrantBackend::default();
        backend.insert(grant("a", "run-1", NOW)).await.unwrap();
        assert!(backend.revoke("a").await.unwrap());
        assert!(backend.get("a").await.unwrap().is_none());
        assert!(backend.is_revoked("a").await.unwrap());
        assert_eq!(backend.len().await.unwrap(), 0);
        assert!(!backend.revoke("a").await.unwrap());
    }

    #[tokio::test]
    async fn revoke_unknown_id_records_nothing() {
        let backend = MemoryGrantBackend::default();
        assert!(!backend.revoke("missing").await.unwrap());
        assert!(!backend.is_revoked("missing").await.unwrap());
    }

    #[tokio::test]
    async fn remove_does_not_keep_a_tombstone() {
        let backend = MemoryGrantBackend::default();
        backend.insert(grant("a", "run-1", NOW)).await.unwrap();
        assert!(backend.remove("a").await.unwrap());
        assert!(!backend.is_revoked("a").await.unwrap());
    }

    #[tokio::test]
    async fn purge_expired_drops_tombstones_at_or_before_now() {
        let backend = MemoryGrantBackend::default();
        backend.insert(grant("a", "run-1", NOW + 10)).await.unwrap();
        backend.insert(grant("b", "run-1", NOW + 30)).await.unwrap();
        assert_eq!(backend.revoke_run("run-1").await.unwrap(), 2);
        assert_eq!(backend.purge_expired(NOW + 10).await.unwrap(), 0);
        assert!(!backend.is_revoked("a").await.unwrap());
        assert!(backend.is_revoked("b").await.unwrap());
        assert_eq!(backend.purge_expired(NOW + 30).await.unwrap(), 0);
        assert!(!backend.is_revoked("b").await.unwrap());
    }

    #[tokio::test]
    async fn purge_expired_drops_grants_at_or_before_now() {
        let backend = MemoryGrantBackend::default();
        backend.insert(grant("a", "run-1", NOW + 10)).await.unwrap();
        backend.insert(grant("b", "run-1", NOW + 20)).await.unwrap();
        backend.insert(grant("c", "run-1", NOW + 21)).await.unwrap();
        assert_eq!(backend.purge_expired(NOW + 20).await.unwrap(), 2);
        assert_eq!(backend.len().await.unwrap(), 1);
        assert!(backend.get("c").await.unwrap().is_some());
        assert_eq!(backend.purge_expired(NOW + 20).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn clones_share_grants() {
        let backend = MemoryGrantBackend::default();
        let clone = backend.clone();
        backend.insert(grant("a", "run-1", NOW)).await.unwrap();
        assert_eq!(clone.len().await.unwrap(), 1);
        assert!(clone.remove("a").await.unwrap());
        assert_eq!(backend.len().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn registry_never_stores_the_token() {
        let backend = Arc::new(MemoryGrantBackend::default());
        let registry = AuthProxyRegistry::with_backend(backend.clone());
        let issued = registry
            .issue(
                TokenRequest {
                    run_id: "run-1".to_string(),
                    step: "review".to_string(),
                    expires_at: NOW + 600,
                    credential: ProxyCredential::new(
                        CredentialKind::OauthToken,
                        "sk-ant-oat01-test".to_string(),
                    )
                    .into(),
                },
                NOW,
            )
            .await
            .unwrap();
        let grants = backend.grants();
        let keys: Vec<&String> = grants.keys().collect();
        assert_eq!(keys, vec![&token_id(&issued.token)]);
        assert_ne!(keys[0], &issued.token);
        let stored = format!("{:?}", grants.values().collect::<Vec<_>>());
        assert!(!stored.contains(&issued.token), "{stored}");
        assert!(!stored.contains("sk-ant-oat01-test"), "{stored}");
    }
}
