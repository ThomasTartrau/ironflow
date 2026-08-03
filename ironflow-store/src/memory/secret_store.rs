//! [`SecretStore`] trait implementation for [`InMemoryStore`].

#[cfg(feature = "secret-store")]
use std::collections::hash_map::Entry;

#[cfg(feature = "secret-store")]
use chrono::Utc;
#[cfg(feature = "secret-store")]
use uuid::Uuid;

#[cfg(feature = "secret-store")]
use tracing::error;

#[cfg(feature = "secret-store")]
use crate::crypto::{decrypt, encrypt, join_versions};
use crate::entities::{
    KeyVersionStatus, Page, RotationBatch, RotationRequest, Secret, SecretMetadata,
};
use crate::error::StoreError;
use crate::secret_store::SecretStore;
use crate::store::StoreFuture;

use super::InMemoryStore;

#[cfg(feature = "secret-store")]
impl InMemoryStore {
    /// The configured key ring, or a [`StoreError::Crypto`] naming what is missing.
    fn require_key_ring(&self) -> Result<&crate::crypto::KeyRing, StoreError> {
        self.key_ring
            .as_deref()
            .ok_or_else(|| StoreError::Crypto("no master key configured".to_string()))
    }
}

impl SecretStore for InMemoryStore {
    fn get_secret(&self, key: &str) -> StoreFuture<'_, Option<Secret>> {
        let key = key.to_string();
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;

                let state = self.state.read().await;
                let Some(encrypted) = state.secrets.get(&key) else {
                    return Ok(None);
                };

                let master_key = ring.key_for(encrypted.key_version).ok_or_else(|| {
                    StoreError::Crypto(format!(
                        "secret {key:?} uses key version {} which is not configured",
                        encrypted.key_version
                    ))
                })?;

                let plaintext = decrypt(master_key, &encrypted.encrypted_value, &encrypted.nonce)
                    .map_err(|e| StoreError::Crypto(e.to_string()))?;

                let value = String::from_utf8(plaintext)
                    .map_err(|e| StoreError::Crypto(format!("invalid UTF-8: {e}")))?;

                Ok(Some(Secret {
                    id: encrypted.id,
                    key: encrypted.key.clone(),
                    value,
                    created_at: encrypted.created_at,
                    updated_at: encrypted.updated_at,
                }))
            }
            #[cfg(not(feature = "secret-store"))]
            {
                let _ = key;
                Err(StoreError::Crypto(
                    "secret-store feature not enabled".to_string(),
                ))
            }
        })
    }

    fn set_secret(&self, key: &str, value: &str) -> StoreFuture<'_, Secret> {
        let key = key.to_string();
        let value = value.to_string();
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;

                let (encrypted_value, nonce) = encrypt(ring.active_key(), value.as_bytes())
                    .map_err(|e| StoreError::Crypto(e.to_string()))?;
                let key_version = ring.active_version();

                let now = Utc::now();
                let mut state = self.state.write().await;

                let entry = state.secrets.entry(key.clone());
                let encrypted = match entry {
                    Entry::Occupied(mut occ) => {
                        let existing = occ.get_mut();
                        existing.encrypted_value = encrypted_value;
                        existing.nonce = nonce;
                        existing.key_version = key_version;
                        existing.updated_at = now;
                        existing.clone()
                    }
                    Entry::Vacant(vac) => {
                        let new = super::EncryptedSecret {
                            id: Uuid::now_v7(),
                            key: key.clone(),
                            encrypted_value,
                            nonce,
                            key_version,
                            created_at: now,
                            updated_at: now,
                        };
                        vac.insert(new.clone());
                        new
                    }
                };

                Ok(Secret {
                    id: encrypted.id,
                    key: encrypted.key,
                    value,
                    created_at: encrypted.created_at,
                    updated_at: encrypted.updated_at,
                })
            }
            #[cfg(not(feature = "secret-store"))]
            {
                let _ = (key, value);
                Err(StoreError::Crypto(
                    "secret-store feature not enabled".to_string(),
                ))
            }
        })
    }

    fn delete_secret(&self, key: &str) -> StoreFuture<'_, bool> {
        let key = key.to_string();
        Box::pin(async move {
            let mut state = self.state.write().await;
            Ok(state.secrets.remove(&key).is_some())
        })
    }

    fn list_secret_keys(&self, prefix: &str) -> StoreFuture<'_, Vec<String>> {
        let prefix = prefix.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            let keys: Vec<String> = state
                .secrets
                .keys()
                .filter(|k| k.starts_with(&prefix))
                .cloned()
                .collect();
            Ok(keys)
        })
    }

    fn list_secrets(
        &self,
        prefix: &str,
        page: u32,
        per_page: u32,
    ) -> StoreFuture<'_, Page<SecretMetadata>> {
        let prefix = prefix.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            let mut metadata: Vec<SecretMetadata> = state
                .secrets
                .values()
                .filter(|s| s.key.starts_with(&prefix))
                .map(|s| SecretMetadata {
                    id: s.id,
                    key: s.key.clone(),
                    created_at: s.created_at,
                    updated_at: s.updated_at,
                })
                .collect();

            metadata.sort_by(|a, b| a.key.cmp(&b.key));

            let total = metadata.len() as u64;
            let offset = ((page.saturating_sub(1)) as usize) * (per_page as usize);
            let items: Vec<SecretMetadata> = metadata
                .into_iter()
                .skip(offset)
                .take(per_page as usize)
                .collect();

            Ok(Page {
                items,
                total,
                page,
                per_page,
            })
        })
    }

    fn secret_key_status(&self) -> StoreFuture<'_, KeyVersionStatus> {
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;

                let state = self.state.read().await;
                let mut in_use: Vec<i32> = state.secrets.values().map(|s| s.key_version).collect();
                in_use.sort_unstable();
                in_use.dedup();

                Ok(KeyVersionStatus {
                    active: ring.active_version(),
                    configured: ring.versions(),
                    missing: ring.missing_versions(&in_use),
                    retirable: ring.retirable_versions(&in_use),
                    in_use,
                })
            }
            #[cfg(not(feature = "secret-store"))]
            {
                Err(StoreError::Crypto(
                    "secret-store feature not enabled".to_string(),
                ))
            }
        })
    }

    fn rotate_secrets(&self, request: RotationRequest) -> StoreFuture<'_, RotationBatch> {
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let ring = self.require_key_ring()?;
                let target = ring.key_for(request.to_version).ok_or_else(|| {
                    StoreError::Crypto(format!(
                        "target key version {} is not configured (available: {})",
                        request.to_version,
                        join_versions(&ring.versions())
                    ))
                })?;

                let batch_size = request.effective_batch_size() as usize;
                let mut state = self.state.write().await;

                // The Postgres implementation walks the stock ordered by id;
                // mirror that here so both behave the same under a cursor.
                let mut candidates: Vec<(Uuid, String)> = state
                    .secrets
                    .values()
                    .filter(|s| s.key_version != request.to_version)
                    .filter(|s| request.after_id.is_none_or(|after| s.id > after))
                    .map(|s| (s.id, s.key.clone()))
                    .collect();
                candidates.sort_unstable_by_key(|(id, _)| *id);

                let remaining = candidates.len().saturating_sub(batch_size) as u64;
                let mut rotated = 0u64;
                let mut failed = 0u64;
                let mut last_id = None;

                for (id, key) in candidates.into_iter().take(batch_size) {
                    last_id = Some(id);

                    let Some(secret) = state.secrets.get(&key) else {
                        continue;
                    };
                    let source_version = secret.key_version;

                    let plaintext = match ring
                        .key_for(source_version)
                        .ok_or_else(|| format!("key version {source_version} is not configured"))
                        .and_then(|source| {
                            decrypt(source, &secret.encrypted_value, &secret.nonce)
                                .map_err(|e| e.to_string())
                        }) {
                        Ok(plaintext) => plaintext,
                        Err(reason) => {
                            failed += 1;
                            error!(
                                secret_key = %key,
                                key_version = source_version,
                                reason = %reason,
                                "secret rotation failed"
                            );
                            continue;
                        }
                    };

                    let (encrypted_value, nonce) = match encrypt(target, &plaintext) {
                        Ok(pair) => pair,
                        Err(e) => {
                            failed += 1;
                            error!(
                                secret_key = %key,
                                reason = %e,
                                "secret rotation failed"
                            );
                            continue;
                        }
                    };

                    let entry = state
                        .secrets
                        .get_mut(&key)
                        .expect("secret was present a moment ago under the same write lock");
                    entry.encrypted_value = encrypted_value;
                    entry.nonce = nonce;
                    entry.key_version = request.to_version;
                    // updated_at is deliberately untouched: re-encryption is
                    // not a change to the secret itself.
                    rotated += 1;
                }

                Ok(RotationBatch {
                    to_version: request.to_version,
                    rotated,
                    failed,
                    remaining,
                    last_id,
                })
            }
            #[cfg(not(feature = "secret-store"))]
            {
                let _ = request;
                Err(StoreError::Crypto(
                    "secret-store feature not enabled".to_string(),
                ))
            }
        })
    }
}

#[cfg(all(test, feature = "secret-store"))]
mod tests {
    use crate::crypto::MasterKey;
    use crate::memory::InMemoryStore;
    use crate::secret_store::SecretStore;

    fn test_store() -> InMemoryStore {
        let key = MasterKey::from_bytes(&[42u8; 32]).unwrap();
        let mut store = InMemoryStore::new();
        store.set_master_key(key);
        store
    }

    #[tokio::test]
    async fn set_and_get_secret() {
        let store = test_store();
        let secret = store.set_secret("my/key", "my-value").await.unwrap();
        assert_eq!(secret.key, "my/key");
        assert_eq!(secret.value, "my-value");

        let fetched = store.get_secret("my/key").await.unwrap().unwrap();
        assert_eq!(fetched.value, "my-value");
        assert_eq!(fetched.id, secret.id);
    }

    #[tokio::test]
    async fn get_missing_secret_returns_none() {
        let store = test_store();
        let result = store.get_secret("does/not/exist").await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn set_secret_updates_existing() {
        let store = test_store();
        let first = store.set_secret("token", "v1").await.unwrap();
        let second = store.set_secret("token", "v2").await.unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.value, "v2");

        let fetched = store.get_secret("token").await.unwrap().unwrap();
        assert_eq!(fetched.value, "v2");
    }

    #[tokio::test]
    async fn delete_existing_secret() {
        let store = test_store();
        store.set_secret("to-delete", "val").await.unwrap();

        let deleted = store.delete_secret("to-delete").await.unwrap();
        assert!(deleted);

        let fetched = store.get_secret("to-delete").await.unwrap();
        assert!(fetched.is_none());
    }

    #[tokio::test]
    async fn delete_missing_secret_returns_false() {
        let store = test_store();
        let deleted = store.delete_secret("nope").await.unwrap();
        assert!(!deleted);
    }

    #[tokio::test]
    async fn list_keys_with_prefix() {
        let store = test_store();
        store.set_secret("wf/inbox/token_a", "a").await.unwrap();
        store.set_secret("wf/inbox/token_b", "b").await.unwrap();
        store.set_secret("wf/veille/token_c", "c").await.unwrap();

        let mut keys = store.list_secret_keys("wf/inbox/").await.unwrap();
        keys.sort();
        assert_eq!(keys, vec!["wf/inbox/token_a", "wf/inbox/token_b"]);
    }

    #[tokio::test]
    async fn list_keys_empty_prefix_returns_all() {
        let store = test_store();
        store.set_secret("a", "1").await.unwrap();
        store.set_secret("b", "2").await.unwrap();

        let keys = store.list_secret_keys("").await.unwrap();
        assert_eq!(keys.len(), 2);
    }

    #[tokio::test]
    async fn operations_without_master_key_fail() {
        let store = InMemoryStore::new();
        let err = store.get_secret("key").await.unwrap_err();
        assert!(err.to_string().contains("no master key"));

        let err = store.set_secret("key", "val").await.unwrap_err();
        assert!(err.to_string().contains("no master key"));
    }

    #[tokio::test]
    async fn secret_value_is_encrypted_at_rest() {
        let store = test_store();
        store
            .set_secret("sensitive", "plaintext-value")
            .await
            .unwrap();

        let state = store.state.read().await;
        let encrypted = state.secrets.get("sensitive").unwrap();
        let as_str = String::from_utf8(encrypted.encrypted_value.clone());
        assert!(
            as_str.is_err() || as_str.unwrap() != "plaintext-value",
            "value must be encrypted at rest"
        );
    }

    #[tokio::test]
    async fn set_secret_with_empty_value() {
        let store = test_store();
        let secret = store.set_secret("empty", "").await.unwrap();
        assert_eq!(secret.value, "");

        let fetched = store.get_secret("empty").await.unwrap().unwrap();
        assert_eq!(fetched.value, "");
    }

    #[tokio::test]
    async fn list_secrets_paginated() {
        let store = test_store();
        store.set_secret("a/1", "v").await.unwrap();
        store.set_secret("a/2", "v").await.unwrap();
        store.set_secret("a/3", "v").await.unwrap();
        store.set_secret("b/1", "v").await.unwrap();

        let page = store.list_secrets("a/", 1, 2).await.unwrap();
        assert_eq!(page.total, 3);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].key, "a/1");
        assert_eq!(page.items[1].key, "a/2");

        let page2 = store.list_secrets("a/", 2, 2).await.unwrap();
        assert_eq!(page2.items.len(), 1);
        assert_eq!(page2.items[0].key, "a/3");
    }

    // -- Key versioning and rotation ------------------------------------

    use crate::crypto::KeyRing;
    use crate::entities::RotationRequest;

    fn hex_key(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    /// A store whose ring holds versions 1 and 2, with `active` encrypting.
    fn two_version_store(active: i32) -> InMemoryStore {
        let spec = format!("1:{},2:{}", hex_key(0xaa), hex_key(0xbb));
        let mut store = InMemoryStore::new();
        store.set_key_ring(KeyRing::from_spec(&spec, Some(active)).unwrap());
        store
    }

    /// Swap the ring in place, keeping the stored secrets untouched.
    fn rekey(store: &mut InMemoryStore, active: i32) {
        let spec = format!("1:{},2:{}", hex_key(0xaa), hex_key(0xbb));
        store.set_key_ring(KeyRing::from_spec(&spec, Some(active)).unwrap());
    }

    #[tokio::test]
    async fn set_master_key_is_legacy_version_one() {
        let store = test_store();
        store.set_secret("k", "v").await.unwrap();

        let status = store.secret_key_status().await.unwrap();
        assert_eq!(status.active, 1);
        assert_eq!(status.configured, vec![1]);
        assert_eq!(status.in_use, vec![1]);
        assert!(status.is_consistent());
    }

    #[tokio::test]
    async fn new_secret_uses_active_version() {
        let store = two_version_store(2);
        store.set_secret("k", "v").await.unwrap();

        let status = store.secret_key_status().await.unwrap();
        assert_eq!(status.in_use, vec![2]);
        assert_eq!(status.retirable, vec![1]);
    }

    #[tokio::test]
    async fn secret_written_with_old_version_stays_readable() {
        let mut store = two_version_store(1);
        store.set_secret("legacy", "old-value").await.unwrap();

        rekey(&mut store, 2);
        store.set_secret("fresh", "new-value").await.unwrap();

        assert_eq!(
            store.get_secret("legacy").await.unwrap().unwrap().value,
            "old-value"
        );
        assert_eq!(
            store.get_secret("fresh").await.unwrap().unwrap().value,
            "new-value"
        );

        let status = store.secret_key_status().await.unwrap();
        assert_eq!(status.in_use, vec![1, 2]);
        assert!(status.retirable.is_empty());
    }

    #[tokio::test]
    async fn secret_key_status_reports_missing_version() {
        let mut store = two_version_store(2);
        store.set_secret("k", "v").await.unwrap();

        // Drop version 2 from the ring while a secret still uses it.
        store.set_key_ring(KeyRing::from_spec(&format!("1:{}", hex_key(0xaa)), Some(1)).unwrap());

        let status = store.secret_key_status().await.unwrap();
        assert_eq!(status.missing, vec![2]);
        assert!(!status.is_consistent());
    }

    #[tokio::test]
    async fn secret_with_unconfigured_version_fails_to_read() {
        let mut store = two_version_store(2);
        store.set_secret("k", "v").await.unwrap();

        store.set_key_ring(KeyRing::from_spec(&format!("1:{}", hex_key(0xaa)), Some(1)).unwrap());

        let err = store.get_secret("k").await.unwrap_err().to_string();
        assert!(err.contains("key version 2"));
        assert!(err.contains("not configured"));
    }

    #[tokio::test]
    async fn rotate_moves_whole_stock_to_target_version() {
        let mut store = two_version_store(1);
        for i in 0..5 {
            store
                .set_secret(&format!("k{i}"), &format!("v{i}"))
                .await
                .unwrap();
        }
        rekey(&mut store, 2);

        let batch = store.rotate_secrets(RotationRequest::new(2)).await.unwrap();
        assert_eq!(batch.rotated, 5);
        assert_eq!(batch.failed, 0);
        assert_eq!(batch.remaining, 0);
        assert!(batch.is_complete());

        let status = store.secret_key_status().await.unwrap();
        assert_eq!(status.in_use, vec![2]);
        assert_eq!(status.retirable, vec![1]);

        for i in 0..5 {
            assert_eq!(
                store
                    .get_secret(&format!("k{i}"))
                    .await
                    .unwrap()
                    .unwrap()
                    .value,
                format!("v{i}")
            );
        }
    }

    #[tokio::test]
    async fn rotate_is_idempotent() {
        let mut store = two_version_store(1);
        store.set_secret("k", "v").await.unwrap();
        rekey(&mut store, 2);

        let first = store.rotate_secrets(RotationRequest::new(2)).await.unwrap();
        assert_eq!(first.rotated, 1);

        let second = store.rotate_secrets(RotationRequest::new(2)).await.unwrap();
        assert_eq!(second.rotated, 0);
        assert_eq!(second.remaining, 0);
        assert!(second.last_id.is_none());
        assert!(second.is_complete());

        assert_eq!(store.get_secret("k").await.unwrap().unwrap().value, "v");
    }

    #[tokio::test]
    async fn rotate_on_empty_store_is_a_noop() {
        let store = two_version_store(2);
        let batch = store.rotate_secrets(RotationRequest::new(2)).await.unwrap();

        assert_eq!(batch.rotated, 0);
        assert_eq!(batch.failed, 0);
        assert_eq!(batch.remaining, 0);
        assert!(batch.last_id.is_none());
        assert!(batch.is_complete());
    }

    #[tokio::test]
    async fn rotate_batch_by_batch_with_cursor() {
        let mut store = two_version_store(1);
        for i in 0..3 {
            store
                .set_secret(&format!("k{i}"), &format!("v{i}"))
                .await
                .unwrap();
        }
        rekey(&mut store, 2);

        let mut cursor = None;
        let mut batches = 0;
        loop {
            let mut request = RotationRequest::new(2).with_batch_size(1);
            if let Some(id) = cursor {
                request = request.after(id);
            }
            let batch = store.rotate_secrets(request).await.unwrap();
            batches += 1;
            assert_eq!(batch.rotated, 1);

            // Every secret stays readable between batches, whichever
            // version it currently sits on.
            for i in 0..3 {
                assert_eq!(
                    store
                        .get_secret(&format!("k{i}"))
                        .await
                        .unwrap()
                        .unwrap()
                        .value,
                    format!("v{i}")
                );
            }

            if batch.is_complete() {
                break;
            }
            cursor = batch.last_id;
        }

        assert_eq!(batches, 3);
        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![2]);
    }

    #[tokio::test]
    async fn interrupted_rotation_resumes_from_scratch() {
        let mut store = two_version_store(1);
        for i in 0..4 {
            store
                .set_secret(&format!("k{i}"), &format!("v{i}"))
                .await
                .unwrap();
        }
        rekey(&mut store, 2);

        // First half, then the operator kills the CLI: the cursor is lost.
        let partial = store
            .rotate_secrets(RotationRequest::new(2).with_batch_size(2))
            .await
            .unwrap();
        assert_eq!(partial.rotated, 2);
        assert_eq!(partial.remaining, 2);

        // Mixed stock, everything still readable.
        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![1, 2]);
        for i in 0..4 {
            assert_eq!(
                store
                    .get_secret(&format!("k{i}"))
                    .await
                    .unwrap()
                    .unwrap()
                    .value,
                format!("v{i}")
            );
        }

        // Restarting without a cursor only picks up the leftovers.
        let resumed = store.rotate_secrets(RotationRequest::new(2)).await.unwrap();
        assert_eq!(resumed.rotated, 2);
        assert_eq!(resumed.remaining, 0);
        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![2]);
    }

    #[tokio::test]
    async fn rotate_preserves_identity_and_timestamps() {
        let mut store = two_version_store(1);
        let before = store.set_secret("k", "v").await.unwrap();
        rekey(&mut store, 2);

        store.rotate_secrets(RotationRequest::new(2)).await.unwrap();

        let after = store.get_secret("k").await.unwrap().unwrap();
        assert_eq!(after.id, before.id);
        assert_eq!(after.key, before.key);
        assert_eq!(after.value, before.value);
        assert_eq!(after.created_at, before.created_at);
        assert_eq!(after.updated_at, before.updated_at);
    }

    #[tokio::test]
    async fn rotate_changes_the_ciphertext() {
        let mut store = two_version_store(1);
        store.set_secret("k", "v").await.unwrap();

        let before = {
            let state = store.state.read().await;
            state.secrets["k"].encrypted_value.clone()
        };

        rekey(&mut store, 2);
        store.rotate_secrets(RotationRequest::new(2)).await.unwrap();

        let state = store.state.read().await;
        assert_ne!(state.secrets["k"].encrypted_value, before);
        assert_eq!(state.secrets["k"].key_version, 2);
    }

    #[tokio::test]
    async fn rotate_to_unconfigured_version_fails_without_touching_anything() {
        let store = two_version_store(1);
        store.set_secret("k", "v").await.unwrap();

        let err = store
            .rotate_secrets(RotationRequest::new(9))
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("key version 9"));
        assert!(err.contains("1, 2"));

        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![1]);
        assert_eq!(store.get_secret("k").await.unwrap().unwrap().value, "v");
    }

    #[tokio::test]
    async fn rotate_without_key_ring_fails() {
        let store = InMemoryStore::new();
        let err = store
            .rotate_secrets(RotationRequest::new(1))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no master key"));

        let err = store.secret_key_status().await.unwrap_err();
        assert!(err.to_string().contains("no master key"));
    }

    #[tokio::test]
    async fn rotate_batch_size_is_clamped() {
        let mut store = two_version_store(1);
        for i in 0..3 {
            store.set_secret(&format!("k{i}"), "v").await.unwrap();
        }
        rekey(&mut store, 2);

        // 0 clamps up to 1, so exactly one secret moves.
        let batch = store
            .rotate_secrets(RotationRequest::new(2).with_batch_size(0))
            .await
            .unwrap();
        assert_eq!(batch.rotated, 1);
        assert_eq!(batch.remaining, 2);
    }

    #[tokio::test]
    async fn undecryptable_secret_is_counted_as_failed_and_skipped() {
        let mut store = two_version_store(1);
        store.set_secret("broken", "v").await.unwrap();

        // Version 1 is still configured, but with the wrong key material --
        // the operator pasted a different key under the same version.
        let wrong = format!("1:{},2:{}", hex_key(0xcc), hex_key(0xbb));
        store.set_key_ring(KeyRing::from_spec(&wrong, Some(2)).unwrap());

        let batch = store.rotate_secrets(RotationRequest::new(2)).await.unwrap();
        assert_eq!(batch.rotated, 0);
        assert_eq!(batch.failed, 1);
        // The cursor moved past the failed row, so the loop terminates.
        assert!(batch.last_id.is_some());
        assert_eq!(batch.remaining, 0);
        assert!(batch.is_complete());

        // The row is left exactly as it was: no partial write.
        let state = store.state.read().await;
        assert_eq!(state.secrets["broken"].key_version, 1);
    }

    #[tokio::test]
    async fn failed_secret_does_not_block_the_rest_of_the_batch() {
        let mut store = two_version_store(1);
        store.set_secret("a", "va").await.unwrap();
        store.set_secret("b", "vb").await.unwrap();
        store.set_secret("c", "vc").await.unwrap();

        // Move "b" to version 2 so it survives the version 1 key swap below.
        rekey(&mut store, 2);
        store.set_secret("b", "vb").await.unwrap();

        // Now version 1 holds the wrong material: "a" and "c" become
        // undecryptable, "b" is untouched.
        let wrong = format!("1:{},2:{}", hex_key(0xcc), hex_key(0xbb));
        store.set_key_ring(KeyRing::from_spec(&wrong, Some(2)).unwrap());

        let batch = store.rotate_secrets(RotationRequest::new(2)).await.unwrap();
        assert_eq!(batch.failed, 2);
        assert_eq!(batch.rotated, 0);
        assert_eq!(batch.remaining, 0);

        assert_eq!(store.get_secret("b").await.unwrap().unwrap().value, "vb");
    }

    #[tokio::test]
    async fn rotate_backwards_to_an_older_version() {
        let mut store = two_version_store(2);
        store.set_secret("k", "v").await.unwrap();
        rekey(&mut store, 1);

        let batch = store.rotate_secrets(RotationRequest::new(1)).await.unwrap();
        assert_eq!(batch.rotated, 1);
        assert_eq!(store.secret_key_status().await.unwrap().in_use, vec![1]);
        assert_eq!(store.get_secret("k").await.unwrap().unwrap().value, "v");
    }
}
