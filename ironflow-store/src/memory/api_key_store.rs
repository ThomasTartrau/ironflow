//! [`ApiKeyStore`] trait implementation for [`InMemoryStore`].

use chrono::Utc;
use uuid::Uuid;

use crate::api_key_store::ApiKeyStore;
use crate::entities::{ApiKey, ApiKeyUpdate, NewApiKey};
use crate::error::StoreError;
use crate::store::StoreFuture;

use super::InMemoryStore;

impl ApiKeyStore for InMemoryStore {
    fn create_api_key(&self, req: NewApiKey) -> StoreFuture<'_, ApiKey> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let now = Utc::now();
            let id = Uuid::now_v7();
            let key = ApiKey {
                id,
                user_id: req.user_id,
                name: req.name,
                key_hash: req.key_hash,
                key_prefix: req.key_prefix,
                scopes: req.scopes,
                is_active: true,
                expires_at: req.expires_at,
                last_used_at: None,
                created_at: now,
                updated_at: now,
                rate_limit_override: req.rate_limit_override,
            };
            state.api_keys.insert(id, key.clone());
            Ok(key)
        })
    }

    fn find_api_key_by_prefix(&self, prefix: &str) -> StoreFuture<'_, Option<ApiKey>> {
        let prefix = prefix.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state
                .api_keys
                .values()
                .find(|k| k.key_prefix == prefix && k.is_active)
                .cloned())
        })
    }

    fn find_api_key_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<ApiKey>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.api_keys.get(&id).cloned())
        })
    }

    fn list_api_keys_by_user(&self, user_id: Uuid) -> StoreFuture<'_, Vec<ApiKey>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let keys: Vec<ApiKey> = state
                .api_keys
                .values()
                .filter(|k| k.user_id == user_id)
                .cloned()
                .collect();
            Ok(keys)
        })
    }

    fn update_api_key(&self, id: Uuid, update: ApiKeyUpdate) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let key = state
                .api_keys
                .get_mut(&id)
                .ok_or(StoreError::Database(format!("API key {id} not found")))?;
            if let Some(name) = update.name {
                key.name = name;
            }
            if let Some(scopes) = update.scopes {
                key.scopes = scopes;
            }
            if let Some(is_active) = update.is_active {
                key.is_active = is_active;
            }
            if let Some(expires_at) = update.expires_at {
                key.expires_at = expires_at;
            }
            if let Some(rate_limit_override) = update.rate_limit_override {
                key.rate_limit_override = rate_limit_override;
            }
            key.updated_at = Utc::now();
            Ok(())
        })
    }

    fn touch_api_key(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            if let Some(key) = state.api_keys.get_mut(&id) {
                key.last_used_at = Some(Utc::now());
            }
            Ok(())
        })
    }

    fn delete_api_key(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            state.api_keys.remove(&id);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::entities::ApiKeyScope;

    use super::*;

    #[tokio::test]
    async fn create_api_key_returns_active_key() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let key = store
            .create_api_key(NewApiKey {
                user_id,
                name: "production".to_string(),
                key_hash: "bcrypt_hash".to_string(),
                key_prefix: "sk_prod_abc123".to_string(),
                scopes: vec![ApiKeyScope::WorkflowsRead],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        assert_eq!(key.user_id, user_id);
        assert_eq!(key.name, "production");
        assert!(key.is_active);
        assert!(key.last_used_at.is_none());
    }

    #[tokio::test]
    async fn find_api_key_by_prefix_existing() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let created = store
            .create_api_key(NewApiKey {
                user_id,
                name: "test".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_test_xyz".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        let found = store
            .find_api_key_by_prefix("sk_test_xyz")
            .await
            .unwrap()
            .expect("key should exist");

        assert_eq!(found.id, created.id);
    }

    #[tokio::test]
    async fn find_api_key_by_prefix_missing_returns_none() {
        let store = InMemoryStore::new();
        let found = store
            .find_api_key_by_prefix("sk_nonexistent")
            .await
            .unwrap();

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_api_key_by_prefix_inactive_returns_none() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let created = store
            .create_api_key(NewApiKey {
                user_id,
                name: "test".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_inactive".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        // Deactivate the key
        store
            .update_api_key(
                created.id,
                ApiKeyUpdate {
                    name: None,
                    scopes: None,
                    is_active: Some(false),
                    expires_at: None,
                    rate_limit_override: None,
                },
            )
            .await
            .unwrap();

        let found = store.find_api_key_by_prefix("sk_inactive").await.unwrap();

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_api_key_by_id_existing() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let created = store
            .create_api_key(NewApiKey {
                user_id,
                name: "test".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_test".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        let found = store
            .find_api_key_by_id(created.id)
            .await
            .unwrap()
            .expect("key should exist");

        assert_eq!(found.id, created.id);
    }

    #[tokio::test]
    async fn find_api_key_by_id_missing_returns_none() {
        let store = InMemoryStore::new();
        let found = store.find_api_key_by_id(Uuid::now_v7()).await.unwrap();

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn list_api_keys_by_user_returns_only_user_keys() {
        let store = InMemoryStore::new();
        let user1 = Uuid::now_v7();
        let user2 = Uuid::now_v7();

        store
            .create_api_key(NewApiKey {
                user_id: user1,
                name: "key1".to_string(),
                key_hash: "hash1".to_string(),
                key_prefix: "sk_1".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        store
            .create_api_key(NewApiKey {
                user_id: user1,
                name: "key2".to_string(),
                key_hash: "hash2".to_string(),
                key_prefix: "sk_2".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        store
            .create_api_key(NewApiKey {
                user_id: user2,
                name: "key3".to_string(),
                key_hash: "hash3".to_string(),
                key_prefix: "sk_3".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        let user1_keys = store.list_api_keys_by_user(user1).await.unwrap();
        assert_eq!(user1_keys.len(), 2);
        assert!(user1_keys.iter().all(|k| k.user_id == user1));
    }

    #[tokio::test]
    async fn update_api_key_name() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let created = store
            .create_api_key(NewApiKey {
                user_id,
                name: "old-name".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_test".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        store
            .update_api_key(
                created.id,
                ApiKeyUpdate {
                    name: Some("new-name".to_string()),
                    scopes: None,
                    is_active: None,
                    expires_at: None,
                    rate_limit_override: None,
                },
            )
            .await
            .unwrap();

        let updated = store.find_api_key_by_id(created.id).await.unwrap().unwrap();

        assert_eq!(updated.name, "new-name");
    }

    #[tokio::test]
    async fn update_api_key_scopes() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let created = store
            .create_api_key(NewApiKey {
                user_id,
                name: "test".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_test".to_string(),
                scopes: vec![ApiKeyScope::WorkflowsRead],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        let new_scopes = vec![ApiKeyScope::WorkflowsRead, ApiKeyScope::RunsWrite];

        store
            .update_api_key(
                created.id,
                ApiKeyUpdate {
                    name: None,
                    scopes: Some(new_scopes.clone()),
                    is_active: None,
                    expires_at: None,
                    rate_limit_override: None,
                },
            )
            .await
            .unwrap();

        let updated = store.find_api_key_by_id(created.id).await.unwrap().unwrap();

        assert_eq!(updated.scopes, new_scopes);
    }

    #[tokio::test]
    async fn touch_api_key_updates_last_used() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let created = store
            .create_api_key(NewApiKey {
                user_id,
                name: "test".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_test".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        assert!(created.last_used_at.is_none());

        store.touch_api_key(created.id).await.unwrap();

        let touched = store.find_api_key_by_id(created.id).await.unwrap().unwrap();

        assert!(touched.last_used_at.is_some());
    }

    #[tokio::test]
    async fn delete_api_key() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let created = store
            .create_api_key(NewApiKey {
                user_id,
                name: "test".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_test".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        store.delete_api_key(created.id).await.unwrap();

        let found = store.find_api_key_by_id(created.id).await.unwrap();

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_api_key_is_idempotent() {
        let store = InMemoryStore::new();
        let result = store.delete_api_key(Uuid::now_v7()).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn create_api_key_with_rate_limit_override() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let key = store
            .create_api_key(NewApiKey {
                user_id,
                name: "high-volume".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_hv".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: Some(500),
            })
            .await
            .unwrap();

        assert_eq!(key.rate_limit_override, Some(500));

        let found = store.find_api_key_by_id(key.id).await.unwrap().unwrap();
        assert_eq!(found.rate_limit_override, Some(500));
    }

    #[tokio::test]
    async fn update_api_key_rate_limit_override() {
        let store = InMemoryStore::new();
        let user_id = Uuid::now_v7();

        let created = store
            .create_api_key(NewApiKey {
                user_id,
                name: "test".to_string(),
                key_hash: "hash".to_string(),
                key_prefix: "sk_rl".to_string(),
                scopes: vec![],
                expires_at: None,
                rate_limit_override: None,
            })
            .await
            .unwrap();

        assert!(created.rate_limit_override.is_none());

        store
            .update_api_key(
                created.id,
                ApiKeyUpdate {
                    name: None,
                    scopes: None,
                    is_active: None,
                    expires_at: None,
                    rate_limit_override: Some(Some(200)),
                },
            )
            .await
            .unwrap();

        let updated = store.find_api_key_by_id(created.id).await.unwrap().unwrap();
        assert_eq!(updated.rate_limit_override, Some(200));

        store
            .update_api_key(
                created.id,
                ApiKeyUpdate {
                    name: None,
                    scopes: None,
                    is_active: None,
                    expires_at: None,
                    rate_limit_override: Some(None),
                },
            )
            .await
            .unwrap();

        let cleared = store.find_api_key_by_id(created.id).await.unwrap().unwrap();
        assert!(cleared.rate_limit_override.is_none());
    }
}
