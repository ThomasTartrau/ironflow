//! [`UserStore`] trait implementation for [`InMemoryStore`].

use chrono::Utc;
use uuid::Uuid;

use crate::entities::{NewUser, Page, User};
use crate::error::StoreError;
use crate::store::StoreFuture;
use crate::user_store::UserStore;

use super::InMemoryStore;

impl UserStore for InMemoryStore {
    fn create_user(&self, req: NewUser) -> StoreFuture<'_, User> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            let email_exists = state.users.values().any(|u| u.email == req.email);
            if email_exists {
                return Err(StoreError::DuplicateEmail(req.email));
            }

            let username_exists = state.users.values().any(|u| u.username == req.username);
            if username_exists {
                return Err(StoreError::DuplicateUsername(req.username));
            }

            let is_admin = req.is_admin.unwrap_or(state.users.is_empty());

            let now = Utc::now();
            let user = User {
                id: Uuid::now_v7(),
                email: req.email,
                username: req.username,
                password_hash: req.password_hash,
                is_admin,
                created_at: now,
                updated_at: now,
            };

            state.users.insert(user.id, user.clone());
            Ok(user)
        })
    }

    fn find_user_by_email(&self, email: &str) -> StoreFuture<'_, Option<User>> {
        let email = email.to_string();
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.users.values().find(|u| u.email == email).cloned())
        })
    }

    fn find_user_by_id(&self, id: Uuid) -> StoreFuture<'_, Option<User>> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.users.get(&id).cloned())
        })
    }

    fn count_users(&self) -> StoreFuture<'_, u64> {
        Box::pin(async move {
            let state = self.state.read().await;
            Ok(state.users.len() as u64)
        })
    }

    fn list_users(&self, page: u32, per_page: u32) -> StoreFuture<'_, Page<User>> {
        Box::pin(async move {
            let state = self.state.read().await;
            let mut users: Vec<User> = state.users.values().cloned().collect();
            users.sort_by_key(|u| std::cmp::Reverse(u.created_at));

            let total = users.len() as u64;
            let offset = ((page.saturating_sub(1)) as usize) * (per_page as usize);
            let items: Vec<User> = users
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

    fn delete_user(&self, id: Uuid) -> StoreFuture<'_, ()> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            state
                .users
                .remove(&id)
                .ok_or(StoreError::UserNotFound(id))?;
            Ok(())
        })
    }

    fn update_user_role(&self, id: Uuid, is_admin: bool) -> StoreFuture<'_, User> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            let user = state
                .users
                .get_mut(&id)
                .ok_or(StoreError::UserNotFound(id))?;
            user.is_admin = is_admin;
            user.updated_at = Utc::now();
            Ok(user.clone())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn new_user(email: &str, username: &str) -> NewUser {
        NewUser {
            email: email.to_string(),
            username: username.to_string(),
            password_hash: "argon2hash".to_string(),
            is_admin: None,
        }
    }

    #[tokio::test]
    async fn create_user_first_user_is_admin() {
        let store = InMemoryStore::new();
        let user = store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        assert_eq!(user.email, "alice@example.com");
        assert_eq!(user.username, "alice");
        assert_eq!(user.password_hash, "argon2hash");
        assert!(user.is_admin);
    }

    #[tokio::test]
    async fn create_user_second_user_is_not_admin() {
        let store = InMemoryStore::new();
        store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let second = store
            .create_user(new_user("bob@example.com", "bob"))
            .await
            .unwrap();

        assert!(!second.is_admin);
    }

    #[tokio::test]
    async fn create_user_explicit_admin_flag() {
        let store = InMemoryStore::new();
        // First user but explicitly set to non-admin
        let first = store
            .create_user(NewUser {
                email: "alice@example.com".to_string(),
                username: "alice".to_string(),
                password_hash: "argon2hash".to_string(),
                is_admin: Some(false),
            })
            .await
            .unwrap();
        assert!(!first.is_admin);

        // Second user but explicitly set to admin
        let second = store
            .create_user(NewUser {
                email: "bob@example.com".to_string(),
                username: "bob".to_string(),
                password_hash: "argon2hash".to_string(),
                is_admin: Some(true),
            })
            .await
            .unwrap();
        assert!(second.is_admin);
    }

    #[tokio::test]
    async fn create_user_duplicate_email_returns_error() {
        let store = InMemoryStore::new();
        store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let err = store
            .create_user(new_user("alice@example.com", "bob"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, StoreError::DuplicateEmail(ref e) if e == "alice@example.com"),
            "expected DuplicateEmail, got: {err}"
        );
    }

    #[tokio::test]
    async fn create_user_duplicate_username_returns_error() {
        let store = InMemoryStore::new();
        store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let err = store
            .create_user(new_user("bob@example.com", "alice"))
            .await
            .unwrap_err();

        assert!(
            matches!(err, StoreError::DuplicateUsername(ref u) if u == "alice"),
            "expected DuplicateUsername, got: {err}"
        );
    }

    #[tokio::test]
    async fn find_user_by_email_existing() {
        let store = InMemoryStore::new();
        let created = store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let found = store
            .find_user_by_email("alice@example.com")
            .await
            .unwrap()
            .expect("user should exist");

        assert_eq!(found.id, created.id);
        assert_eq!(found.email, "alice@example.com");
    }

    #[tokio::test]
    async fn find_user_by_email_missing_returns_none() {
        let store = InMemoryStore::new();
        let found = store
            .find_user_by_email("nobody@example.com")
            .await
            .unwrap();

        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_user_by_id_existing() {
        let store = InMemoryStore::new();
        let created = store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        let found = store
            .find_user_by_id(created.id)
            .await
            .unwrap()
            .expect("user should exist");

        assert_eq!(found.email, "alice@example.com");
        assert_eq!(found.username, "alice");
    }

    #[tokio::test]
    async fn find_user_by_id_missing_returns_none() {
        let store = InMemoryStore::new();
        let found = store.find_user_by_id(Uuid::now_v7()).await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn count_users_empty_store() {
        let store = InMemoryStore::new();
        assert_eq!(store.count_users().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn count_users_with_users() {
        let store = InMemoryStore::new();
        store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();
        store
            .create_user(new_user("bob@example.com", "bob"))
            .await
            .unwrap();
        assert_eq!(store.count_users().await.unwrap(), 2);
    }

    #[tokio::test]
    async fn list_users_paginated() {
        let store = InMemoryStore::new();
        for i in 0..5 {
            store
                .create_user(new_user(
                    &format!("user{i}@example.com"),
                    &format!("user{i}"),
                ))
                .await
                .unwrap();
        }

        let page = store.list_users(1, 2).await.unwrap();
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.total, 5);
        assert_eq!(page.page, 1);
        assert_eq!(page.per_page, 2);

        let page2 = store.list_users(3, 2).await.unwrap();
        assert_eq!(page2.items.len(), 1);
    }

    #[tokio::test]
    async fn list_users_empty_store() {
        let store = InMemoryStore::new();
        let page = store.list_users(1, 20).await.unwrap();
        assert!(page.items.is_empty());
        assert_eq!(page.total, 0);
    }

    #[tokio::test]
    async fn delete_user_existing() {
        let store = InMemoryStore::new();
        let user = store
            .create_user(new_user("alice@example.com", "alice"))
            .await
            .unwrap();

        store.delete_user(user.id).await.unwrap();
        assert_eq!(store.count_users().await.unwrap(), 0);
    }

    #[tokio::test]
    async fn delete_user_not_found() {
        let store = InMemoryStore::new();
        let err = store.delete_user(Uuid::now_v7()).await.unwrap_err();
        assert!(matches!(err, StoreError::UserNotFound(_)));
    }

    #[tokio::test]
    async fn update_user_role_promote() {
        let store = InMemoryStore::new();
        // Create two users so the first gets auto-admin
        let _admin = store
            .create_user(new_user("admin@example.com", "admin"))
            .await
            .unwrap();
        let member = store
            .create_user(new_user("member@example.com", "member"))
            .await
            .unwrap();
        assert!(!member.is_admin);

        let promoted = store.update_user_role(member.id, true).await.unwrap();
        assert!(promoted.is_admin);
    }

    #[tokio::test]
    async fn update_user_role_demote() {
        let store = InMemoryStore::new();
        let admin = store
            .create_user(new_user("admin@example.com", "admin"))
            .await
            .unwrap();
        assert!(admin.is_admin);

        let demoted = store.update_user_role(admin.id, false).await.unwrap();
        assert!(!demoted.is_admin);
    }

    #[tokio::test]
    async fn update_user_role_not_found() {
        let store = InMemoryStore::new();
        let err = store
            .update_user_role(Uuid::now_v7(), true)
            .await
            .unwrap_err();
        assert!(matches!(err, StoreError::UserNotFound(_)));
    }
}
