//! [`SecretStore`] trait implementation for [`InMemoryStore`].

#[cfg(feature = "secret-store")]
use std::collections::hash_map::Entry;

#[cfg(feature = "secret-store")]
use chrono::Utc;
#[cfg(feature = "secret-store")]
use uuid::Uuid;

#[cfg(feature = "secret-store")]
use crate::crypto::{decrypt, encrypt};
use crate::entities::{Page, Secret, SecretMetadata};
use crate::error::StoreError;
use crate::secret_store::SecretStore;
use crate::store::StoreFuture;

use super::InMemoryStore;

impl SecretStore for InMemoryStore {
    fn get_secret(&self, key: &str) -> StoreFuture<'_, Option<Secret>> {
        let key = key.to_string();
        Box::pin(async move {
            #[cfg(feature = "secret-store")]
            {
                let master_key = self
                    .master_key
                    .as_ref()
                    .ok_or_else(|| StoreError::Crypto("no master key configured".to_string()))?;

                let state = self.state.read().await;
                let Some(encrypted) = state.secrets.get(&key) else {
                    return Ok(None);
                };

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
                let master_key = self
                    .master_key
                    .as_ref()
                    .ok_or_else(|| StoreError::Crypto("no master key configured".to_string()))?;

                let (encrypted_value, nonce) = encrypt(master_key, value.as_bytes())
                    .map_err(|e| StoreError::Crypto(e.to_string()))?;

                let now = Utc::now();
                let mut state = self.state.write().await;

                let entry = state.secrets.entry(key.clone());
                let encrypted = match entry {
                    Entry::Occupied(mut occ) => {
                        let existing = occ.get_mut();
                        existing.encrypted_value = encrypted_value;
                        existing.nonce = nonce;
                        existing.updated_at = now;
                        existing.clone()
                    }
                    Entry::Vacant(vac) => {
                        let new = super::EncryptedSecret {
                            id: Uuid::now_v7(),
                            key: key.clone(),
                            encrypted_value,
                            nonce,
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
}
